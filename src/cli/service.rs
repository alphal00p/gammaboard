//! Bootstrap and ownership for the combined control/API service.
use super::{
    instance::{Instance, private_permissions},
    shared,
};
use anyhow::{Context, Result, ensure};

use gammaboard::{
    PgStore,
    api::{node_launch, nodes},
    core::RunReadStore,
    local_control as control,
    runtime_context::{RuntimeContext, RuntimeOverrides},
    stores::RunLifecycleState,
};
use gammaboard_client::{Client, protocol::Request};
use serde_json::{Value, json};
use sqlx::Connection;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::{fd::AsRawFd, unix::process::CommandExt},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub use gammaboard_client::cli::Start as StartArgs;
#[derive(Debug)]
pub struct StopArgs {
    pub wait: bool,
    pub timeout: Option<Duration>,
}
pub fn lock(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    ensure!(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "another operation owns {}",
        path.display()
    );
    Ok(file)
}
pub fn atomic_json(path: &Path, value: &impl serde::Serialize, overwrite: bool) -> Result<()> {
    let parent = path.parent().context("output needs a parent directory")?;
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    writeln!(file)?;
    file.as_file().sync_all()?;
    if overwrite {
        file.persist(path).map_err(|e| e.error)?;
    } else {
        file.persist_noclobber(path).map_err(|e| e.error)?;
    }
    Ok(())
}
fn client(instance: &Instance) -> Result<Client> {
    let host = match instance.server.api_host {
        std::net::IpAddr::V4(ip) if ip.is_unspecified() => "127.0.0.1".into(),
        std::net::IpAddr::V6(ip) => format!(
            "[{}]",
            if ip.is_unspecified() {
                std::net::Ipv6Addr::LOCALHOST
            } else {
                ip
            }
        ),
        ip => ip.to_string(),
    };
    Client::new(
        &format!("http://{host}:{}", instance.server.api_port),
        gammaboard::server::bootstrap_token(&instance.server)?,
    )
}
fn directories(instance: &Instance) -> Result<()> {
    fs::create_dir_all(&instance.paths.state_dir)?;
    private_permissions(&instance.paths.state_dir, 0o700)?;
    fs::create_dir_all(&instance.paths.log_dir)?;
    Ok(())
}
fn db_lock(instance: &Instance) -> Result<Option<File>> {
    if !instance.managed_database {
        return Ok(None);
    }
    let data = Path::new(&instance.runtime.local_postgres.data_dir);
    fs::create_dir_all(data.parent().context("database path needs a parent")?)?;
    Ok(Some(lock(&data.with_extension("gammaboard.lock"))?))
}
async fn provision(instance: &Instance) -> Result<()> {
    if instance.managed_database {
        gammaboard::local_db::start_postgres_cluster(
            &instance.runtime.local_postgres,
            &instance.runtime.database.url,
        )?;
        gammaboard::local_db::create_instance_database(&instance.runtime.database.url).await?;
    }
    Ok(())
}
pub async fn start(instance: &Instance, args: StartArgs, allowed: bool) -> Result<()> {
    instance.require_local_processes(allowed, "start local services or workers")?;
    directories(instance)?;
    if args.component.as_deref() == Some("db") {
        ensure!(
            instance.managed_database,
            "this instance uses an external database; there is no local PostgreSQL service to start"
        );
        ensure!(
            !args.foreground
                && !args.api_only
                && args.workers.is_none()
                && !args.restore_workers
                && !args.wait,
            "start db only provisions PostgreSQL; use start for workers or foreground service ownership"
        );
        if let Ok(status) = client(instance)?
            .request_with_timeout(
                &Request::Status {},
                args.timeout.min(Duration::from_secs(30)),
            )
            .await
            && status["configuration"] == instance.redacted()?
        {
            if status["state"] == "closing" {
                wait_for_owner_exit(instance, args.timeout).await?;
            } else {
                ensure!(
                    status["state"] == "running" && status["ready"] == true,
                    "service is starting or draining; wait before starting its database"
                );
                shared::print_json(&json!({"database_started":true}));
                return Ok(());
            }
        }
        let _guard = lock(&instance.paths.state_dir.join("service.lock"))?;
        let _db = db_lock(instance)?;
        provision(instance).await?;
        shared::print_json(&json!({"database_started":true}));
        return Ok(());
    }
    if args.foreground {
        return serve(instance.clone(), args, allowed).await;
    }
    let _startup = lock(&instance.paths.state_dir.join("startup.lock"))?;
    let api = client(instance)?;
    let status = api
        .request_with_timeout(
            &Request::Status {},
            args.timeout.min(Duration::from_secs(30)),
        )
        .await;
    let closing = status.as_ref().is_ok_and(|s| s["state"] == "closing")
        || (status.is_err()
            && fs::read(instance.paths.state_dir.join("status.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .is_some_and(|s| s["state"] == "closing"));
    if closing {
        wait_for_owner_exit(instance, args.timeout).await?;
    } else if let Ok(status) = status {
        ensure!(
            status["state"] == "running" && status["ready"] == true,
            "service is starting or draining; wait for it before starting more work"
        );
        ensure!(
            status["configuration"] == instance.redacted()?,
            "instance configuration changed; restart the existing service using its original configuration"
        );
        let mut ids: Vec<String> = Vec::new();
        if let Some(count) = args.workers {
            let receipt = api
                .request(&Request::WorkerLaunch {
                    count: Some(count as usize),
                    toml: None,
                })
                .await?;
            ids.extend(serde_json::from_value::<Vec<String>>(
                receipt["request_ids"].clone(),
            )?);
        }
        if args.restore_workers {
            let receipt = api.request(&Request::WorkerRestore {}).await?;
            ids.extend(serde_json::from_value::<Vec<String>>(
                receipt["request_ids"].clone(),
            )?);
        }
        if args.wait {
            api.wait_for_requests(&ids, args.timeout).await?;
        }
        let mut status = api.request(&Request::Status {}).await?;
        status["request_ids"] = json!(ids);
        shared::print_json(&status);
        return Ok(());
    }
    // An unreachable live owner must not be replaced based on a failed HTTP probe.
    drop(
        lock(&instance.paths.state_dir.join("service.lock"))
            .context("service owner is alive but its API is unavailable; inspect service.log")?,
    );
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(instance.paths.log_dir.join("service.log"))?;
    let mut command = Command::new(gammaboard::runtime_context::self_executable()?);
    command.arg("--instance").arg(&instance.source);
    if allowed {
        command.arg("--allow-local-processes");
    }
    command.args(["start", "--foreground"]);
    if args.component.as_deref() == Some("server") {
        command.arg("server");
    }
    if args.api_only {
        command.arg("--api-only");
    }
    if args.restore_workers {
        command.arg("--restore-workers");
    }
    if let Some(count) = args.workers {
        command.args(["--workers", &count.to_string()]);
    }
    command.args(["--timeout", &format!("{}ms", args.timeout.as_millis())]);
    command
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    let deadline = Instant::now() + args.timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            anyhow::bail!(
                "service exited ({status}); inspect {}",
                instance.paths.log_dir.join("service.log").display()
            );
        }
        if let Ok(status) = api
            .request_with_timeout(
                &Request::Status {},
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(30)),
            )
            .await
            && status["ready"] == true
        {
            if args.wait {
                let ids: Vec<String> = serde_json::from_value(status["request_ids"].clone())?;
                api.wait_for_requests(&ids, args.timeout).await?;
            }
            shared::print_json(&status);
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "service readiness timed out; startup may still be running; inspect status/service.log before retrying"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
/// Owner-side restart waits for actual process teardown, beyond application acknowledgment.
pub async fn stop(instance: &Instance, args: StopArgs) -> Result<()> {
    directories(instance)?;
    if let Ok(_guard) = lock(&instance.paths.state_dir.join("service.lock")) {
        return Ok(());
    }
    let api = client(instance)?;
    let started = Instant::now();
    let accepted = api
        .request_with_timeout(
            &Request::Stop {
                wait: args.wait,
                timeout_ms: args.timeout.map(|d| d.as_millis() as u64),
            },
            args.timeout
                .unwrap_or(Duration::from_secs(30))
                .min(Duration::from_secs(30)),
        )
        .await?;
    let id = accepted["stop_id"]
        .as_str()
        .context("missing shutdown ID")?;
    api.wait_for_stop(
        id,
        args.timeout.map(|t| t.saturating_sub(started.elapsed())),
    )
    .await?;
    wait_for_owner_exit(
        instance,
        args.timeout
            .map(|t| t.saturating_sub(started.elapsed()))
            .unwrap_or(Duration::from_secs(30)),
    )
    .await
}
async fn wait_for_owner_exit(instance: &Instance, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(_guard) = lock(&instance.paths.state_dir.join("service.lock")) {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "application shutdown acknowledged, but process exit was not confirmed"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
pub fn stop_database(instance: &Instance) -> Result<()> {
    ensure!(
        instance.managed_database,
        "the deployment does not own this database"
    );
    directories(instance)?;
    let _owner = lock(&instance.paths.state_dir.join("service.lock"))
        .context("stop the instance before stopping its database")?;
    let _db = db_lock(instance)?;
    gammaboard::local_db::stop_instance_database(&instance.runtime.local_postgres)?;
    shared::print_json(&json!({"database_stopped":true}));
    Ok(())
}

struct Service {
    instance: Instance,
    store: PgStore,
    owner: String,
    request_ids: Vec<String>,
    ready: bool,
    exit: bool,
    stop_id: Option<String>,
    stop_result: Option<Value>,
    drain: Option<Drain>,
    error: Option<String>,
}
struct Drain {
    wait: bool,
    runs: Vec<i32>,
    deadline: Option<Instant>,
}
impl Service {
    fn status(&self) -> Value {
        json!({"name":self.instance.name,"state":if self.exit{"closing"}else if self.drain.is_some(){"draining"}else{"running"},
            "pid":std::process::id(),"ready":self.ready,"server":!self.exit,"database_owned":self.instance.managed_database,
            "error":self.error,"stop_id":self.stop_id,"stop_result":self.stop_result,"configuration":self.instance.redacted().ok(),
            "request_ids":self.request_ids,"waiting_for_runs":self.drain.as_ref().map(|d|&d.runs)})
    }
    async fn publish(&self) -> Result<()> {
        let status = self.status();
        control::publish(&self.store, &self.owner, &status).await?;
        atomic_json(
            &self.instance.paths.state_dir.join("status.json"),
            &status,
            true,
        )
    }
    async fn request_stop(&mut self) -> Result<()> {
        control::request_stop(&self.store, None, false, None).await?;
        Ok(())
    }
    async fn tick(&mut self) -> Result<()> {
        let status = control::status(&self.store).await?;
        if let Some(id) = status["stop_id"].as_str()
            && self.stop_id.as_deref() != Some(id)
        {
            let request = &status["shutdown_request"];
            ensure!(
                request["component"].is_null(),
                "component shutdown is no longer supported; stop the instance"
            );
            let elapsed = request["requested_at"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|t| {
                    (chrono::Utc::now() - t.with_timezone(&chrono::Utc))
                        .num_milliseconds()
                        .max(0) as u64
                })
                .unwrap_or(0);
            self.stop_id = Some(id.into());
            self.stop_result = None;
            self.error = None;
            self.drain = Some(Drain {
                wait: request["wait"] == true,
                runs: serde_json::from_value(request["runs"].clone())?,
                deadline: request["timeout_ms"]
                    .as_u64()
                    .map(|ms| Instant::now() + Duration::from_millis(ms.saturating_sub(elapsed))),
            });
        }
        control::process_exports(&self.store, &self.owner).await?;
        let Some(drain) = &self.drain else {
            return self.publish().await;
        };
        let mut unfinished = Vec::new();
        let mut failed = Vec::new();
        for id in &drain.runs {
            if let Some(run) = self.store.get_run_progress(*id).await? {
                match run.lifecycle_state {
                    RunLifecycleState::Completed => {}
                    RunLifecycleState::Failed => failed.push(*id),
                    _ => unfinished.push(*id),
                }
            }
        }
        if !unfinished.is_empty() {
            if drain.deadline.is_some_and(|d| Instant::now() >= d) {
                self.finish(json!({"success":false,"message":format!("timed out waiting for runs {unfinished:?}; instance left running")})).await?;
            }
            return self.publish().await;
        }
        if drain.wait
            && control::exports(&self.store)
                .await?
                .iter()
                .any(|e| e.error.is_some())
        {
            self.finish(json!({"success":false,"message":"result export failed; instance retained for repair and retry"})).await?;
            return self.publish().await;
        }
        if control::require_external_cleanup(&self.store, self.stop_id.as_deref().unwrap()).await? {
            self.store.save_workers_for_restart().await?;
            if drain.deadline.is_some_and(|d| Instant::now() >= d) {
                self.finish(json!({"success":false,"phase":"cleanup_timeout","message":"external cleanup was not acknowledged; services retained and new work remains fenced"})).await?;
            }
            return self.publish().await;
        }
        let stopped =
            nodes::stop_workers_for_restart(&self.store, self.instance.server.cleanup.clone())
                .await?;
        if stopped.sampler_drain_timed_out || stopped.node_stop_timed_out {
            self.finish(json!({"success":false,"message":"worker shutdown did not complete; database retained"})).await?;
            return self.publish().await;
        }
        control::process_exports(&self.store, &self.owner).await?;
        self.finish(
            json!({"success":failed.is_empty(),"phase":"ready_to_exit","failed_runs":failed}),
        )
        .await?;
        self.ready = false;
        self.exit = true;
        self.publish().await
    }
    async fn finish(&mut self, result: Value) -> Result<()> {
        control::finish_stop(
            &self.store,
            self.stop_id
                .as_deref()
                .context("missing shutdown identity")?,
            &result,
        )
        .await?;
        self.stop_result = Some(result);
        self.drain = None;
        Ok(())
    }
}
async fn serve(instance: Instance, args: StartArgs, allowed: bool) -> Result<()> {
    let _owner_lock = lock(&instance.paths.state_dir.join("service.lock"))?;
    let _db_lock = db_lock(&instance)?;
    if args.component.as_deref() != Some("server") {
        provision(&instance).await?;
    }
    let snapshot = instance
        .paths
        .state_dir
        .join("deployments")
        .join(uuid::Uuid::new_v4().to_string());
    instance.write_snapshot(&snapshot)?;
    let runtime = RuntimeContext::load(snapshot.join("runtime.toml"), RuntimeOverrides::default())?
        .with_local_processes(allowed);
    let store = gammaboard::init_pg_store(&instance.runtime.database.url, 8).await?;
    shared::init_cli_tracing(&store, &instance.runtime, false)?;
    let mut guard = sqlx::PgConnection::connect(&instance.runtime.database.url).await?;
    let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock(714831593420019::bigint)")
        .fetch_one(&mut guard)
        .await?;
    ensure!(acquired, "another service owns this database");
    let owner = uuid::Uuid::new_v4().to_string();
    control::attach(&store, &owner).await?;
    // One-way import for instances created before exports moved into PostgreSQL.
    let legacy = instance.paths.state_dir.join("state.json");
    if legacy.is_file() {
        let value: Value = serde_json::from_slice(&fs::read(legacy)?)?;
        for export in value["exports"].as_array().into_iter().flatten() {
            control::import_export(&store, &serde_json::from_value(export.clone())?).await?;
        }
    }

    let mut service = Service {
        instance: instance.clone(),
        store: store.clone(),
        owner,
        request_ids: Vec::new(),
        ready: false,
        exit: false,
        stop_id: None,
        stop_result: None,
        drain: None,
        error: None,
    };
    let mut api = gammaboard::server::spawn(
        store.clone(),
        instance.server.clone(),
        runtime.clone().with_local_processes(false),
        !args.api_only,
        Some(instance.clone()),
    )
    .await?;
    service.publish().await?;
    let local = !instance.runtime.externally_managed && instance.server.allow_local_node_spawn;
    // The service owns local fulfillment. HTTP handlers only persist requests.
    let launcher = if local {
        let store = store.clone();
        let runtime = runtime.clone();
        Some(tokio::spawn(async move {
            loop {
                let pending=sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM node_launch_requests WHERE backend='local' AND state='pending')").fetch_one(store.pool()).await;
                if matches!(pending, Ok(true))
                    && let Ok(_admission) = control::admit(&store).await
                    && let Err(error) = node_launch::resolve_local_requests(&store, &runtime).await
                {
                    tracing::error!(%error,"local launch request failed");
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }))
    } else {
        None
    };
    if let Some(count) = args.workers {
        let _admission = control::admit(&store).await?;
        let receipt = node_launch::launch(
            &store,
            local,
            node_launch::AutoRunNodesRequest {
                toml: None,
                count: Some(count as usize),
                max_start_failures: None,
                args: json!({}),
                name_prefix: None,
            },
        )
        .await?;
        service.request_ids.push(
            receipt["request"]["id"]
                .as_str()
                .context("missing request ID")?
                .into(),
        );
    }
    if args.restore_workers {
        let _admission = control::admit(&store).await?;
        service.request_ids.extend(
            store
                .restore_saved_workers(local)
                .await?
                .into_iter()
                .map(|id| id.to_string()),
        );
    }
    service.ready = true;
    service.publish().await?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let heartbeat_store = store.clone();
    let heartbeat_owner = service.owner.clone();
    let heartbeat = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            if sqlx::query("UPDATE gammaboard_instance_control SET heartbeat_at=now() WHERE singleton AND owner_id=$1").bind(&heartbeat_owner).execute(heartbeat_store.pool()).await.is_err(){break;}
        }
    });
    while !service.exit {
        tokio::select! {
            result=&mut api=>{heartbeat.abort();result??;anyhow::bail!("API service exited unexpectedly; workers and database retained");}
            _=tick.tick()=>{if let Err(error)=service.tick().await {service.error=Some(format!("{error:#}"));service.publish().await?;}}
            _=terminate.recv()=>{service.request_stop().await?;}
            _=interrupt.recv()=>{service.request_stop().await?;}
        }
    }
    // The API remains available briefly to deliver application completion.
    // Its disappearance alone is not a successful shutdown receipt.
    tokio::time::sleep(Duration::from_secs(2)).await;
    if let Some(launcher) = launcher {
        launcher.abort();
        let _ = launcher.await;
    }
    api.abort();
    let _ = api.await;
    heartbeat.abort();
    let _ = heartbeat.await;
    drop(guard);
    store.pool().close().await;
    if instance.managed_database {
        gammaboard::local_db::stop_instance_database(&instance.runtime.local_postgres)?;
    }
    let mut receipt = service.status();
    receipt["state"] = "stopped".into();
    receipt["database_owned"] = false.into();
    atomic_json(
        &instance.paths.state_dir.join("status.json"),
        &receipt,
        true,
    )?;
    Ok(())
}
