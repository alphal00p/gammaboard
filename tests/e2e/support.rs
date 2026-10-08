use super::*;

pub(super) fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_nanos();
    let pid = std::process::id();
    let counter = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{pid}_{nanos}_{counter}")
}

pub(super) fn unused_local_port() -> anyhow::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

pub(super) fn nginx_available() -> bool {
    std::process::Command::new("nginx")
        .arg("-v")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(super) fn resolve_bin_path() -> anyhow::Result<PathBuf> {
    // Cargo builds this executable for integration tests in the selected profile.
    Ok(std::env::var_os("CARGO_BIN_EXE_gammaboard")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_gammaboard"))))
}

pub(super) struct TestDatabase {
    pub(super) admin_url: String,
    pub(super) database_url: String,
    pub(super) database_name: String,
}

impl TestDatabase {
    pub(super) async fn create() -> anyhow::Result<Self> {
        let base_url = std::env::var("GAMMABOARD_TEST_DATABASE_URL").unwrap_or(
            RuntimeConfig::load("ops/local/config/runtime.toml")?
                .database
                .url,
        );

        let mut admin_url = Url::parse(&base_url)?;
        admin_url.set_path("/postgres");

        let database_name = format!("gammaboard_e2e_{}", unique_suffix());
        let admin_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(admin_url.as_str())
            .await?;

        sqlx::query(&format!("CREATE DATABASE \"{database_name}\""))
            .execute(&admin_pool)
            .await?;

        let mut database_url = Url::parse(&base_url)?;
        database_url.set_path(&format!("/{database_name}"));

        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(database_url.as_str())
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        pool.close().await;
        admin_pool.close().await;

        Ok(Self {
            admin_url: admin_url.to_string(),
            database_url: database_url.to_string(),
            database_name,
        })
    }

    pub(super) async fn cleanup(&self) -> anyhow::Result<()> {
        let admin_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&self.admin_url)
            .await?;

        sqlx::query(
            r#"
            SELECT pg_terminate_backend(pid)
            FROM pg_stat_activity
            WHERE datname = $1
              AND pid <> pg_backend_pid()
            "#,
        )
        .bind(&self.database_name)
        .execute(&admin_pool)
        .await?;

        sqlx::query(&format!(
            "DROP DATABASE IF EXISTS \"{}\"",
            self.database_name
        ))
        .execute(&admin_pool)
        .await?;

        admin_pool.close().await;
        Ok(())
    }
}

pub(super) struct FullStackHarness {
    pub(super) db: TestDatabase,
    pub(super) pool: PgPool,
    pub(super) bin_path: PathBuf,
    pub(super) children: Vec<ManagedChild>,
    pub(super) runtime_config_path: PathBuf,
    pub(super) temp_files: Vec<NamedTempFile>,
    pub(super) artifacts: PathBuf,
}

pub(super) struct ManagedChild {
    pub(super) label: String,
    pub(super) child: Child,
}

pub(super) struct RemoveFileOnDrop(pub(super) PathBuf);

impl Drop for RemoveFileOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl FullStackHarness {
    pub(super) async fn new() -> anyhow::Result<Self> {
        let db = TestDatabase::create().await?;
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(&db.database_url)
            .await?;
        let bin_path = resolve_bin_path()?;
        let cli_config = temp_cli_config(&db.database_url, true);
        let runtime_config_path = cli_config.path().to_path_buf();

        let artifacts = std::env::var_os("GAMMABOARD_E2E_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/e2e"))
            .join(&db.database_name);
        std::fs::create_dir_all(&artifacts)?;
        std::fs::copy(cli_config.path(), artifacts.join("runtime.toml"))?;
        eprintln!("E2E artifacts: {}", artifacts.display());
        let temp_files = vec![cli_config];

        Ok(Self {
            db,
            pool,
            bin_path,
            children: Vec::new(),
            runtime_config_path,
            temp_files,
            artifacts,
        })
    }

    fn spawn_logged(&self, label: &str, command: &mut TokioCommand) -> anyhow::Result<Child> {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.artifacts.join(format!("{label}.log")))?;
        command
            .stdout(log.try_clone()?)
            .stderr(log)
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        Ok(command.spawn()?)
    }

    #[cfg(unix)]
    pub(super) fn signal_child(&self, label: &str, signal: i32) -> anyhow::Result<()> {
        let child = self
            .children
            .iter()
            .find(|c| c.label == label)
            .ok_or_else(|| anyhow::anyhow!("missing child {label}"))?;
        let pid = child
            .child
            .id()
            .ok_or_else(|| anyhow::anyhow!("child exited: {label}"))?;
        // Each harness process owns its group, including external runtime children.
        anyhow::ensure!(
            unsafe { libc::kill(-(pid as i32), signal) } == 0,
            "signal {signal} to {label}: {}",
            std::io::Error::last_os_error()
        );
        Ok(())
    }

    pub(super) async fn expire_node(&self, name: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE nodes SET lease_expires_at=now()-interval '1 second' WHERE name=$1")
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub(super) fn cli(&self) -> Command {
        let mut cmd = Command::new(&self.bin_path);
        cmd.arg("--runtime-config").arg(&self.runtime_config_path);
        cmd
    }

    pub(super) async fn start_node(&mut self, node_name: &str) -> anyhow::Result<()> {
        let previous_last_seen: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
            r#"
            SELECT last_seen
            FROM nodes
            WHERE name = $1 AND last_seen IS NOT NULL
            "#,
        )
        .bind(node_name)
        .fetch_optional(&self.pool)
        .await?;

        let mut child = TokioCommand::new(&self.bin_path);
        child
            .arg("--runtime-config")
            .arg(&self.runtime_config_path)
            .arg("node")
            .arg("run")
            .arg("--name")
            .arg(node_name)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        let child = self.spawn_logged(node_name, &mut child)?;
        self.children.push(ManagedChild {
            label: node_name.to_string(),
            child,
        });

        let pool = self.pool.clone();
        let node_name = node_name.to_string();
        self.wait_for(
            format!("node {node_name} registration"),
            Duration::from_secs(10),
            || {
                let pool = pool.clone();
                let node_name = node_name.clone();
                async move {
                    let count: i64 = sqlx::query_scalar(
                        r#"
                            SELECT COUNT(*)
                            FROM nodes
                            WHERE name = $1
                              AND lease_expires_at > now()
                              AND ($2::timestamptz IS NULL OR last_seen > $2)
                            "#,
                    )
                    .bind(&node_name)
                    .bind(previous_last_seen)
                    .fetch_one(&pool)
                    .await?;
                    Ok(count == 1)
                }
            },
        )
        .await
    }

    pub(super) async fn start_nodes(&mut self, node_names: &[&str]) -> anyhow::Result<()> {
        for node_name in node_names {
            self.start_node(node_name).await?;
        }
        Ok(())
    }

    pub(super) async fn start_server(&mut self) -> anyhow::Result<String> {
        let password_hash = hash_password_for_tests("test-password");
        self.start_server_with_auth((&password_hash, "test-session-secret"))
            .await
    }

    pub(super) async fn start_server_with_auth(
        &mut self,
        auth: (&str, &str),
    ) -> anyhow::Result<String> {
        self.start_server_with_auth_and_local_spawn(auth, true)
            .await
    }

    pub(super) async fn start_server_with_auth_and_local_spawn(
        &mut self,
        auth: (&str, &str),
        allow_local_node_spawn: bool,
    ) -> anyhow::Result<String> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let addr = listener.local_addr()?;
        drop(listener);
        let server_config = temp_server_config(
            &addr.ip().to_string(),
            addr.port(),
            "http://localhost:3000",
            false,
            allow_local_node_spawn,
            auth,
        );

        let mut child = TokioCommand::new(&self.bin_path);
        child
            .arg("--runtime-config")
            .arg(&self.runtime_config_path)
            .arg("server")
            .arg("--server-config")
            .arg(server_config.path())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        let child = self.spawn_logged(&format!("server-{addr}"), &mut child)?;
        self.temp_files.push(server_config);
        self.children.push(ManagedChild {
            label: format!("server:{addr}"),
            child,
        });

        let base_url = format!("http://{addr}");
        self.wait_for("server health", Duration::from_secs(15), || {
            let base_url = base_url.clone();
            async move {
                match http_get(&base_url, "/api/health").await {
                    Ok(response) => Ok(response.contains("\"status\":\"ok\"")),
                    Err(_) => Ok(false),
                }
            }
        })
        .await?;

        Ok(base_url)
    }

    pub(super) async fn wait_for<F, Fut>(
        &self,
        label: impl Into<String>,
        timeout: Duration,
        mut condition: F,
    ) -> anyhow::Result<()>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<bool>>,
    {
        let label = label.into();
        tokio::time::timeout(timeout, async {
            loop {
                if condition().await? {
                    return Ok(());
                }
                sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "timed out waiting for {label}; evidence: {}",
                self.artifacts.display()
            )
        })?
    }

    pub(super) async fn node_state(
        &self,
        node_name: &str,
    ) -> anyhow::Result<(Option<i32>, Option<String>, Option<i32>, Option<String>)> {
        let row = sqlx::query(
            r#"
            SELECT
                desired_run_id,
                desired_role,
                active_run_id AS current_run_id,
                active_role AS current_role
            FROM nodes
            WHERE name = $1
            "#,
        )
        .bind(node_name)
        .fetch_one(&self.pool)
        .await?;

        Ok((
            row.try_get("desired_run_id")?,
            row.try_get("desired_role")?,
            row.try_get("current_run_id")?,
            row.try_get("current_role")?,
        ))
    }

    pub(super) async fn run_current_accumulator(
        &self,
        run_id: i32,
    ) -> anyhow::Result<Option<JsonValue>> {
        let accumulator: Option<JsonValue> = sqlx::query_scalar(
            r#"
            SELECT current_observable
            FROM runs
            WHERE id = $1
            "#,
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(accumulator)
    }

    pub(super) async fn run_sampler_checkpoint(
        &self,
        run_id: i32,
    ) -> anyhow::Result<Option<JsonValue>> {
        let checkpoint: Option<JsonValue> = sqlx::query_scalar(
            r#"
            SELECT sampler_checkpoint
            FROM run_sampler_checkpoints
            WHERE run_id = $1
            "#,
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(checkpoint)
    }

    pub(super) async fn run_sample_progress(&self, run_id: i32) -> anyhow::Result<(i64, i64)> {
        let row = sqlx::query(
            r#"
            SELECT nr_produced_samples, nr_completed_samples
            FROM runs
            WHERE id = $1
            "#,
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await?;
        Ok((
            row.try_get("nr_produced_samples")?,
            row.try_get("nr_completed_samples")?,
        ))
    }

    pub(super) async fn run_stage_snapshot_count(&self, run_id: i32) -> anyhow::Result<i64> {
        let count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)
            FROM run_stage_snapshots
            WHERE run_id = $1
            "#,
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    pub(super) async fn persisted_observable_snapshot_count(
        &self,
        run_id: i32,
    ) -> anyhow::Result<i64> {
        let count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)
            FROM persisted_observable_snapshots
            WHERE run_id = $1
            "#,
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    pub(super) async fn latest_task_sampler_grid(
        &self,
        run_id: i32,
        task_name: &str,
    ) -> anyhow::Result<JsonValue> {
        let task_id: i64 = sqlx::query_scalar(
            r#"
            SELECT id
            FROM run_tasks
            WHERE run_id = $1 AND name = $2
            "#,
        )
        .bind(run_id)
        .bind(task_name)
        .fetch_one(&self.pool)
        .await?;

        let sampler_snapshot: JsonValue = sqlx::query_scalar(
            r#"
            SELECT sampler_snapshot
            FROM run_stage_snapshots
            WHERE run_id = $1
              AND task_id = $2
              AND queue_empty = TRUE
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .bind(run_id)
        .bind(task_id)
        .fetch_one(&self.pool)
        .await?;

        let snapshot: SamplerAggregatorSnapshot = serde_json::from_value(sampler_snapshot)?;
        match snapshot {
            SamplerAggregatorSnapshot::HavanaTraining { raw }
            | SamplerAggregatorSnapshot::HavanaInference { raw } => raw
                .get("grid")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("missing havana grid in persisted snapshot")),
            other => Err(anyhow::anyhow!(
                "expected havana sampler snapshot, got {other:?}"
            )),
        }
    }

    pub(super) async fn latest_task_persisted_observable(
        &self,
        run_id: i32,
        task_name: &str,
    ) -> anyhow::Result<JsonValue> {
        let task_id: i64 = sqlx::query_scalar(
            r#"
            SELECT id
            FROM run_tasks
            WHERE run_id = $1 AND name = $2
            "#,
        )
        .bind(run_id)
        .bind(task_name)
        .fetch_one(&self.pool)
        .await?;

        let persisted: JsonValue = sqlx::query_scalar(
            r#"
            SELECT persisted_observable
            FROM persisted_observable_snapshots
            WHERE run_id = $1
              AND task_id = $2
            ORDER BY created_at DESC, id DESC
            LIMIT 1
            "#,
        )
        .bind(run_id)
        .bind(task_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(persisted)
    }

    pub(super) async fn stop_children(&mut self) {
        for managed in &mut self.children {
            #[cfg(unix)]
            if let Some(pid) = managed.child.id() {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            let _ = managed.child.start_kill();
        }
        for managed in &mut self.children {
            let _ = tokio::time::timeout(Duration::from_secs(5), managed.child.wait()).await;
        }
        self.children.clear();
    }

    pub(super) async fn kill_child(&mut self, label: &str) -> anyhow::Result<()> {
        let position = self
            .children
            .iter()
            .position(|managed| managed.label == label)
            .ok_or_else(|| anyhow::anyhow!("missing child process {label}"))?;
        let mut managed = self.children.swap_remove(position);
        #[cfg(unix)]
        if let Some(pid) = managed.child.id() {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        managed.child.start_kill()?;
        let _ = tokio::time::timeout(Duration::from_secs(5), managed.child.wait()).await;
        Ok(())
    }

    pub(super) async fn reap_child(&mut self, label: &str) -> anyhow::Result<()> {
        let position = self
            .children
            .iter()
            .position(|managed| managed.label == label)
            .ok_or_else(|| anyhow::anyhow!("missing child process {label}"))?;
        let mut managed = self.children.swap_remove(position);
        let status = tokio::time::timeout(Duration::from_secs(5), managed.child.wait()).await??;
        if !status.success() {
            anyhow::bail!("child process {label} exited with status {status}");
        }
        Ok(())
    }

    pub(super) async fn reap_children(&mut self, labels: &[&str]) -> anyhow::Result<()> {
        for label in labels {
            self.reap_child(label).await?;
        }
        Ok(())
    }

    #[cfg(unix)]
    pub(super) async fn terminate_child(&mut self, label: &str) -> anyhow::Result<()> {
        let position = self
            .children
            .iter()
            .position(|managed| managed.label == label)
            .ok_or_else(|| anyhow::anyhow!("missing child process {label}"))?;
        let mut managed = self.children.swap_remove(position);
        let pid = managed
            .child
            .id()
            .ok_or_else(|| anyhow::anyhow!("child process {label} has no pid"))?;

        let status = TokioCommand::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("failed to send SIGTERM to child process {label}");
        }

        let status = tokio::time::timeout(Duration::from_secs(10), managed.child.wait()).await??;
        anyhow::ensure!(
            status.success(),
            "child {label} failed graceful shutdown: {status}"
        );
        Ok(())
    }

    pub(super) async fn run_id(&self, name: &str) -> anyhow::Result<i32> {
        let id: i32 = sqlx::query_scalar("SELECT id FROM runs WHERE name = $1")
            .bind(name)
            .fetch_one(&self.pool)
            .await?;
        Ok(id)
    }

    pub(super) fn add_run(&self, config: &NamedTempFile) {
        self.cli()
            .arg("run")
            .arg("create")
            .arg(config.path())
            .assert()
            .success();
    }

    pub(super) fn assign_node(&self, node: &str, role: &str, run_name: &str) {
        self.cli()
            .args(["node", "assign", node, role, run_name])
            .assert()
            .success();
    }

    pub(super) async fn cleanup(&mut self) -> anyhow::Result<()> {
        self.stop_children().await;
        self.temp_files.clear();
        self.pool.close().await;
        self.db.cleanup().await
    }
}

impl Drop for FullStackHarness {
    fn drop(&mut self) {
        for managed in &mut self.children {
            #[cfg(unix)]
            if let Some(pid) = managed.child.id() {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            let _ = managed.child.start_kill();
        }
    }
}

pub(super) fn temp_config(contents: &str) -> NamedTempFile {
    let file = NamedTempFile::new().expect("create temp config");
    std::fs::write(file.path(), contents).expect("write temp config");
    file
}

pub(super) async fn run_havana_training_then_inference(
    harness: &mut FullStackHarness,
    run_name: &str,
    pause_mid_training: bool,
) -> anyhow::Result<(JsonValue, JsonValue)> {
    // Exercise pool recreation and checkpoint determinism with multiple I/O threads.
    let io_threads = if pause_mid_training { 3 } else { 1 };
    let training_samples = 256usize;
    let inference_samples = 64usize;
    let config = temp_config(&format!(
        r#"
name = "{run_name}"

[evaluator]
kind = "unit"
continuous_dims = 2
discrete_dims = 0
# Leave time to request a pause independently of runner polling defaults.
timing = {{ per_sample_seconds = 0.01 }}

[sampler_aggregator_runner_params]
io_threads = {io_threads}
frontend_sync_interval_ms = 50

[[task_queue]]
name = "train-a"
kind = "sample"
stop_condition = {{ max_samples = {training_samples} }}
accumulator = {{ config = "scalar" }}
sampler_aggregator = {{ config = {{ kind = "havana_training", seed = 0, bins = 8, samples_for_update = 8, initial_training_rate = 0.1, final_training_rate = 0.01 }} }}
"#,
    ));

    harness.add_run(&config);
    let run_id = harness.run_id(run_name).await?;

    harness.assign_node("w-2", "evaluator", run_name);
    harness
        .wait_for(
            format!("training evaluator becomes active for {run_name}"),
            Duration::from_secs(15),
            || async {
                let w2 = harness.node_state("w-2").await?;
                Ok(w2.0 == Some(run_id)
                    && w2.1.as_deref() == Some("evaluator")
                    && w2.2 == Some(run_id)
                    && w2.3.as_deref() == Some("evaluator"))
            },
        )
        .await?;

    harness.assign_node("w-1", "sampler_aggregator", run_name);

    if pause_mid_training {
        harness
            .wait_for(
                format!("havana training progresses before pause for {run_name}"),
                Duration::from_secs(30),
                || async {
                    let (nr_produced_samples, nr_completed_samples) =
                        harness.run_sample_progress(run_id).await?;
                    Ok(nr_produced_samples > 0
                        && nr_completed_samples >= 32
                        && nr_completed_samples < training_samples as i64)
                },
            )
            .await?;

        harness
            .cli()
            .args(["run", "pause", run_name])
            .assert()
            .success();

        harness
            .wait_for(
                format!("paused run reconciles nodes down for {run_name}"),
                Duration::from_secs(15),
                || async {
                    let w1 = harness.node_state("w-1").await?;
                    let w2 = harness.node_state("w-2").await?;
                    Ok(w1.0.is_none()
                        && w1.1.is_none()
                        && w1.2.is_none()
                        && w1.3.is_none()
                        && w2.0.is_none()
                        && w2.1.is_none()
                        && w2.2.is_none()
                        && w2.3.is_none())
                },
            )
            .await?;

        let paused_progress = harness.run_sample_progress(run_id).await?;

        harness.assign_node("w-2", "evaluator", run_name);
        harness
            .wait_for(
                format!("resumed training evaluator becomes active for {run_name}"),
                Duration::from_secs(15),
                || async {
                    let w2 = harness.node_state("w-2").await?;
                    Ok(w2.0 == Some(run_id)
                        && w2.1.as_deref() == Some("evaluator")
                        && w2.2 == Some(run_id)
                        && w2.3.as_deref() == Some("evaluator"))
                },
            )
            .await?;
        harness.assign_node("w-1", "sampler_aggregator", run_name);

        harness
            .wait_for(
                format!("training progress advances after resume for {run_name}"),
                Duration::from_secs(30),
                || async {
                    let progress = harness.run_sample_progress(run_id).await?;
                    Ok(progress.0 > paused_progress.0 || progress.1 > paused_progress.1)
                },
            )
            .await?;
    }

    harness
        .wait_for(
            format!("havana training completes for {run_name}"),
            Duration::from_secs(60),
            || async {
                let state: String = sqlx::query_scalar(
                    "SELECT state FROM run_tasks WHERE run_id = $1 AND name = 'train-a'",
                )
                .bind(run_id)
                .fetch_one(&harness.pool)
                .await?;
                Ok(state == "completed")
            },
        )
        .await?;

    let inference_task = temp_config(&format!(
        r#"
[[task_queue]]
name = "infer-a"
kind = "sample"
stop_condition = {{ max_samples = {inference_samples} }}
sampler_aggregator = {{ config = {{ kind = "havana_inference" }} }}
"#,
    ));

    harness
        .cli()
        .args([
            "run",
            "task",
            "append",
            &run_id.to_string(),
            inference_task.path().to_str().expect("task file path"),
        ])
        .assert()
        .success();

    harness.assign_node("w-2", "evaluator", run_name);
    harness
        .wait_for(
            format!("inference evaluator becomes active for {run_name}"),
            Duration::from_secs(15),
            || async {
                let w2 = harness.node_state("w-2").await?;
                Ok(w2.0 == Some(run_id)
                    && w2.1.as_deref() == Some("evaluator")
                    && w2.2 == Some(run_id)
                    && w2.3.as_deref() == Some("evaluator"))
            },
        )
        .await?;
    harness.assign_node("w-1", "sampler_aggregator", run_name);

    harness
        .wait_for(
            format!("havana inference completes for {run_name}"),
            Duration::from_secs(60),
            || async {
                let state: String = sqlx::query_scalar(
                    "SELECT state FROM run_tasks WHERE run_id = $1 AND name = 'infer-a'",
                )
                .bind(run_id)
                .fetch_one(&harness.pool)
                .await?;
                Ok(state == "completed")
            },
        )
        .await?;

    harness
        .wait_for(
            format!("nodes reconcile down after completion for {run_name}"),
            Duration::from_secs(15),
            || async {
                let w1 = harness.node_state("w-1").await?;
                let w2 = harness.node_state("w-2").await?;
                Ok(w1.0.is_none()
                    && w1.1.is_none()
                    && w1.2.is_none()
                    && w1.3.is_none()
                    && w2.0.is_none()
                    && w2.1.is_none()
                    && w2.2.is_none()
                    && w2.3.is_none())
            },
        )
        .await?;

    let training_grid = harness.latest_task_sampler_grid(run_id, "train-a").await?;
    let inference_grid = harness.latest_task_sampler_grid(run_id, "infer-a").await?;
    assert_eq!(inference_samples, 64);
    Ok((training_grid, inference_grid))
}

pub(super) fn temp_server_config(
    host: &str,
    port: u16,
    allowed_origin: &str,
    secure_cookie: bool,
    allow_local_node_spawn: bool,
    auth: (&str, &str),
) -> NamedTempFile {
    let (admin_password_hash, session_secret) = auth;
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let run_templates_dir = manifest_dir.join("resources/templates/runs");
    let task_templates_dir = manifest_dir.join("resources/templates/tasks");
    let node_templates_dir = manifest_dir.join("resources/templates/nodes");
    let contents = format!(
        "api_host = {host:?}\napi_port = {port}\nallowed_origins = [{allowed_origin:?}]\nsecure_cookie = {secure_cookie}\nallow_local_node_spawn = {allow_local_node_spawn}\nrun_templates_dir = {run_templates_dir:?}\ntask_templates_dir = {task_templates_dir:?}\nnode_templates_dir = {node_templates_dir:?}\n\n[auth]\nadmin_password_hash = {admin_password_hash:?}\nsession_secret = {session_secret:?}\n"
    );
    temp_config(&contents)
}

pub(super) fn temp_deploy_server_config(
    frontend_build_dir: &std::path::Path,
    server_config: &std::path::Path,
    frontend_port: u16,
) -> NamedTempFile {
    let contents = std::fs::read_to_string(server_config).expect("read temp server config");
    let contents = format!(
        "{contents}\n[frontend]\nbuild_dir = {:?}\nhost = \"127.0.0.1\"\nport = {frontend_port}\nadvertise_hosts = [\"localhost\"]\naccess_log = false\n\n[database]\nensure_started = false\n\n[cleanup]\nsampler_drain_timeout_seconds = 5\nnode_stop_timeout_seconds = 5\npoll_interval_ms = 100\n",
        frontend_build_dir,
    );
    temp_config(&contents)
}

pub(super) fn temp_frontend_build() -> TempDir {
    let dir = tempfile::tempdir().expect("create temp frontend build");
    std::fs::write(
        dir.path().join("index.html"),
        "<!doctype html><html><body>gammaboard e2e</body></html>",
    )
    .expect("write index.html");
    dir
}

pub(super) fn temp_cli_config(database_url: &str, persist_runtime_logs: bool) -> NamedTempFile {
    let contents = format!(
        "[database]\nurl = {database_url:?}\n\n[tracing]\npersist_runtime_logs = {persist_runtime_logs}\ndb_gammaboard_level = \"info\"\ndb_external_level = \"warn\"\n\n[local_postgres]\ndata_dir = \".postgres\"\nsocket_dir = \"db/socket\"\nlog_file = \".postgres/logfile\"\nmax_connections = 512\n"
    );
    temp_config(&contents)
}

pub(super) async fn http_get(base_url: &str, path: &str) -> anyhow::Result<String> {
    let url = Url::parse(base_url)?.join(path)?;
    let response = reqwest::get(url).await?;
    let body = response.error_for_status()?.text().await?;
    Ok(body)
}

pub(super) async fn http_get_with_cookie(
    base_url: &str,
    path: &str,
    cookie: &str,
) -> anyhow::Result<String> {
    let url = Url::parse(base_url)?.join(path)?;
    let client = reqwest::Client::new();
    let body = client
        .get(url)
        .header("cookie", cookie)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(body)
}

pub(super) async fn login_cookie(base_url: &str) -> anyhow::Result<String> {
    let response = http_post_json(
        base_url,
        "/api/auth/login",
        json!({"password": "test-password"}),
        None,
    )
    .await?;
    anyhow::ensure!(response.status().is_success(), "test login failed");
    response
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or("").to_string())
        .ok_or_else(|| anyhow::anyhow!("missing session cookie"))
}

pub(super) async fn http_post_json(
    base_url: &str,
    path: &str,
    payload: serde_json::Value,
    cookie: Option<&str>,
) -> anyhow::Result<reqwest::Response> {
    let url = Url::parse(base_url)?.join(path)?;
    let client = reqwest::Client::new();
    let mut request = client
        .post(url)
        .header("content-type", "application/json")
        .body(payload.to_string());
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    Ok(request.send().await?)
}

pub(super) fn hash_password_for_tests(password: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .expect("argon2 hash")
        .to_string()
}

pub(super) async fn assert_fresh_controller_duplicate(
    harness: &FullStackHarness,
    source: &str,
) -> anyhow::Result<()> {
    let name = format!("{source}-copy");
    harness
        .cli()
        .args(["run", "duplicate", source, &name])
        .assert()
        .success();
    let duplicate_id = harness.run_id(&name).await?;
    let children: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE parent_run_id=$1")
        .bind(duplicate_id)
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(children, 0, "new controllers must not adopt old child runs");
    let copied_state: (String, Option<JsonValue>) =
        sqlx::query_as("SELECT state, controller_output FROM run_tasks WHERE run_id=$1")
            .bind(duplicate_id)
            .fetch_one(&harness.pool)
            .await?;
    assert_eq!(copied_state, ("pending".into(), None));
    let store = gammaboard::PgStore::new(harness.pool.clone());
    let source_id = harness.run_id(source).await?;
    let source = gammaboard::api::runs::export_run_definition(&store, source_id).await?;
    let copy = gammaboard::api::runs::export_run_definition(&store, duplicate_id).await?;
    let source = gammaboard::api::runs::rename_run_definition(&source, &name)?;
    assert_eq!(
        toml::from_str::<toml::Value>(&source)?,
        toml::from_str::<toml::Value>(&copy)?
    );
    Ok(())
}

const DEFAULT_DISCRETE_MAX_PROB_RATIO: f64 = 30.0;

pub(super) fn build_direct_havana_grid(domain: &Domain, params: &HavanaSamplerParams) -> Grid<f64> {
    match domain {
        Domain::Continuous { dims } => Grid::Continuous(
            ContinuousGrid::new(*dims, params.bins, params.samples_for_update, None, false)
                .unwrap(),
        ),
        Domain::Rectangular {
            discrete_cardinalities,
            continuous_dims,
        } => build_direct_rectangular_havana_grid(*continuous_dims, discrete_cardinalities, params),
        Domain::Discrete { branches, .. } => {
            let bins = branches
                .iter()
                .map(|branch| Some(build_direct_havana_grid(branch.domain.as_ref(), params)))
                .collect();
            Grid::Discrete(DiscreteGrid::new(bins, DEFAULT_DISCRETE_MAX_PROB_RATIO, false).unwrap())
        }
    }
}

pub(super) fn build_direct_rectangular_havana_grid(
    continuous_dims: usize,
    discrete_cardinalities: &[usize],
    params: &HavanaSamplerParams,
) -> Grid<f64> {
    if let Some((&cardinality, tail)) = discrete_cardinalities.split_first() {
        let bins = (0..cardinality)
            .map(|_| {
                Some(build_direct_rectangular_havana_grid(
                    continuous_dims,
                    tail,
                    params,
                ))
            })
            .collect();
        return Grid::Discrete(
            DiscreteGrid::new(bins, DEFAULT_DISCRETE_MAX_PROB_RATIO, false).unwrap(),
        );
    }
    Grid::Continuous(
        ContinuousGrid::new(
            continuous_dims,
            params.bins,
            params.samples_for_update,
            None,
            false,
        )
        .unwrap(),
    )
}

pub(super) fn direct_unit_training_value(_sample: &Sample<f64>) -> f64 {
    1.0
}

pub(super) fn direct_havana_training_rate(
    params: &HavanaSamplerParams,
    samples_ingested: usize,
    stop_training_after_n_samples: usize,
) -> f64 {
    let progress = (samples_ingested.min(stop_training_after_n_samples) as f64)
        / (stop_training_after_n_samples as f64);
    if params.initial_training_rate <= 0.0 || params.final_training_rate <= 0.0 {
        return params.initial_training_rate
            + (params.final_training_rate - params.initial_training_rate) * progress;
    }

    params.initial_training_rate
        * (params.final_training_rate / params.initial_training_rate).powf(progress)
}

pub(super) fn direct_train_havana_grid(
    domain: &Domain,
    params: &HavanaSamplerParams,
    stop_training_after_n_samples: usize,
) -> Grid<f64> {
    let mut grid = build_direct_havana_grid(domain, params);
    let mut rng = Xoshiro256StarStar::seed_from_u64(params.seed);
    let mut samples_ingested = 0usize;

    while samples_ingested < stop_training_after_n_samples {
        let nr_samples = params
            .samples_for_update
            .min(stop_training_after_n_samples - samples_ingested);
        for _ in 0..nr_samples {
            let mut sample = Sample::new();
            grid.sample(&mut rng, &mut sample);
            let eval = direct_unit_training_value(&sample);
            grid.add_training_sample(&sample, eval)
                .expect("direct havana training sample should be valid");
        }
        samples_ingested += nr_samples;
        let training_rate =
            direct_havana_training_rate(params, samples_ingested, stop_training_after_n_samples);
        grid.update(training_rate, training_rate);
    }

    grid
}

pub(super) async fn wait_for_task_failed_and_run_unassigned(
    harness: &FullStackHarness,
    run_id: i32,
    timeout: Duration,
) -> anyhow::Result<()> {
    harness
        .wait_for("task failed and run unassigned", timeout, || async {
            let task: Option<(String, Option<String>)> = sqlx::query_as(
                "SELECT state, failure_reason FROM run_tasks WHERE run_id = $1 AND sequence_nr = 1",
            )
            .bind(run_id)
            .fetch_optional(&harness.pool)
            .await?;
            let Some((state, failure_reason)) = task else {
                return Ok(false);
            };
            let assigned: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM nodes WHERE desired_run_id=$1 OR active_run_id=$1",
            )
            .bind(run_id)
            .fetch_one(&harness.pool)
            .await?;
            Ok(state == "failed" && failure_reason.is_some() && assigned == 0)
        })
        .await
}

pub(super) async fn wait_for_task_state(
    harness: &FullStackHarness,
    run_id: i32,
    expected_state: &str,
    timeout: Duration,
) -> anyhow::Result<()> {
    harness
        .wait_for("task state transition", timeout, || async {
            let state: Option<String> = sqlx::query_scalar(
                "SELECT state FROM run_tasks WHERE run_id = $1 AND sequence_nr = 1",
            )
            .bind(run_id)
            .fetch_optional(&harness.pool)
            .await?;
            Ok(state.as_deref() == Some(expected_state))
        })
        .await
}

pub(super) async fn wait_for_batch_retry_count(
    harness: &FullStackHarness,
    run_id: i32,
    min_retry_count: i32,
    timeout: Duration,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let max_retry: Option<i32> =
            sqlx::query_scalar("SELECT MAX(retry_count) FROM batches WHERE run_id = $1")
                .bind(run_id)
                .fetch_one(&harness.pool)
                .await?;
        if max_retry.unwrap_or(0) >= min_retry_count {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let task_rows: Vec<(i64, String, String, Option<String>)> = sqlx::query_as(
                "SELECT id, name, state, failure_reason FROM run_tasks WHERE run_id = $1 ORDER BY sequence_nr",
            )
            .bind(run_id)
            .fetch_all(&harness.pool)
            .await?;
            let batch_counts: Vec<(String, i64, Option<i32>)> = sqlx::query_as(
                "SELECT status::text, COUNT(*), MAX(retry_count) FROM batches WHERE run_id = $1 GROUP BY status ORDER BY status",
            )
            .bind(run_id)
            .fetch_all(&harness.pool)
            .await?;
            let nodes: Vec<(String, Option<i32>, Option<String>, Option<i32>, Option<String>)> =
                sqlx::query_as(
                    "SELECT name, desired_run_id, desired_role, active_run_id, active_role FROM nodes ORDER BY name",
                )
                .fetch_all(&harness.pool)
                .await?;
            let logs: Vec<(String, String, String, JsonValue)> = sqlx::query_as(
                "SELECT source, level, message, fields FROM runtime_logs WHERE run_id = $1 ORDER BY id DESC LIMIT 12",
            )
            .bind(run_id)
            .fetch_all(&harness.pool)
            .await?;
            anyhow::bail!(
                "timed out waiting for batch retry_count >= {min_retry_count}; max_retry={max_retry:?}; tasks={task_rows:?}; batches={batch_counts:?}; nodes={nodes:?}; logs={logs:?}"
            );
        }
        sleep(Duration::from_millis(100)).await;
    }
}

pub(super) async fn wait_for_failed_batch(
    harness: &FullStackHarness,
    run_id: i32,
    timeout: Duration,
) -> anyhow::Result<()> {
    harness
        .wait_for("failed batch recorded", timeout, || async {
            let failed_batches: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM batches WHERE run_id = $1 AND status = 'failed'",
            )
            .bind(run_id)
            .fetch_one(&harness.pool)
            .await?;
            Ok(failed_batches > 0)
        })
        .await
}
