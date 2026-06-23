use argon2::{
    Argon2,
    password_hash::{PasswordHasher, SaltString, rand_core::OsRng},
};
use assert_cmd::Command;
use gammaboard::Domain;
use gammaboard::api::nodes as node_api;
use gammaboard::config::RuntimeConfig;
use gammaboard::sampling::{
    HavanaSamplerParams, LatentBatch, PdfAdaptationImagePersistedOutput, SamplerAggregatorSnapshot,
};
use predicates::prelude::*;
use rand::SeedableRng;
use rand_xoshiro::Xoshiro256StarStar;
use serde_json::{Value as JsonValue, json};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::collections::HashSet;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use symbolica::numerical_integration::{ContinuousGrid, DiscreteGrid, Grid, Sample};
use tempfile::{NamedTempFile, TempDir};
use tokio::process::{Child, Command as TokioCommand};
use tokio::time::{Instant, sleep};
use url::Url;

static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_nanos();
    let pid = std::process::id();
    let counter = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{pid}_{nanos}_{counter}")
}

fn unused_local_port() -> anyhow::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn nginx_available() -> bool {
    std::process::Command::new("nginx")
        .arg("-v")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn resolve_bin_path() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_gammaboard") {
        return Ok(PathBuf::from(path));
    }

    let current_exe = std::env::current_exe()?;
    let debug_dir = current_exe
        .parent()
        .and_then(|path| path.parent())
        .ok_or_else(|| {
            anyhow::anyhow!("failed to resolve target/debug from current test binary")
        })?;
    let bin_name = if cfg!(windows) {
        "gammaboard.exe"
    } else {
        "gammaboard"
    };
    let candidate = debug_dir.join(bin_name);
    if candidate.is_file() {
        return Ok(candidate);
    }

    anyhow::bail!(
        "missing gammaboard test binary; expected CARGO_BIN_EXE_gammaboard or {}",
        candidate.display()
    );
}

struct TestDatabase {
    admin_url: String,
    database_url: String,
    database_name: String,
}

impl TestDatabase {
    async fn create() -> anyhow::Result<Self> {
        let base_url = RuntimeConfig::load("ops/local/config/runtime.toml")?
            .database
            .url;

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

    async fn cleanup(&self) -> anyhow::Result<()> {
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

struct FullStackHarness {
    db: TestDatabase,
    pool: PgPool,
    bin_path: PathBuf,
    children: Vec<ManagedChild>,
    runtime_config_path: PathBuf,
    temp_files: Vec<NamedTempFile>,
}

struct ManagedChild {
    label: String,
    child: Child,
}

struct RemoveFileOnDrop(PathBuf);

impl Drop for RemoveFileOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl FullStackHarness {
    async fn new() -> anyhow::Result<Self> {
        let db = TestDatabase::create().await?;
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(&db.database_url)
            .await?;
        let bin_path = resolve_bin_path()?;
        let cli_config = temp_cli_config(&db.database_url, true);
        let runtime_config_path = cli_config.path().to_path_buf();

        let mut temp_files = Vec::new();
        temp_files.push(cli_config);

        Ok(Self {
            db,
            pool,
            bin_path,
            children: Vec::new(),
            runtime_config_path,
            temp_files,
        })
    }

    fn cli(&self) -> Command {
        let mut cmd = Command::new(&self.bin_path);
        cmd.arg("--runtime-config").arg(&self.runtime_config_path);
        cmd
    }

    async fn start_node(&mut self, node_name: &str) -> anyhow::Result<()> {
        let previous_last_seen: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
            r#"
            SELECT last_seen
            FROM nodes
            WHERE name = $1
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

        let child = child.spawn()?;
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

    async fn start_nodes(&mut self, node_names: &[&str]) -> anyhow::Result<()> {
        for node_name in node_names {
            self.start_node(node_name).await?;
        }
        Ok(())
    }

    async fn start_server(&mut self) -> anyhow::Result<String> {
        let password_hash = hash_password_for_tests("test-password");
        self.start_server_with_auth((&password_hash, "test-session-secret"))
            .await
    }

    async fn start_server_with_auth(&mut self, auth: (&str, &str)) -> anyhow::Result<String> {
        self.start_server_with_auth_and_local_spawn(auth, true)
            .await
    }

    async fn start_server_with_auth_and_local_spawn(
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

        let child = child.spawn()?;
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

    async fn wait_for<F, Fut>(
        &self,
        label: impl Into<String>,
        timeout: Duration,
        mut condition: F,
    ) -> anyhow::Result<()>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<bool>>,
    {
        let deadline = Instant::now() + timeout;
        let label = label.into();

        loop {
            if condition().await? {
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for {label}");
            }
            sleep(Duration::from_millis(100)).await;
        }
    }

    async fn node_state(
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

    async fn run_current_accumulator(&self, run_id: i32) -> anyhow::Result<Option<JsonValue>> {
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

    async fn run_sampler_checkpoint(&self, run_id: i32) -> anyhow::Result<Option<JsonValue>> {
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

    async fn run_sample_progress(&self, run_id: i32) -> anyhow::Result<(i64, i64)> {
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

    async fn run_stage_snapshot_count(&self, run_id: i32) -> anyhow::Result<i64> {
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

    async fn persisted_observable_snapshot_count(&self, run_id: i32) -> anyhow::Result<i64> {
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

    async fn latest_task_sampler_grid(
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

    async fn latest_task_persisted_observable(
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

    async fn stop_children(&mut self) {
        for managed in &mut self.children {
            let _ = managed.child.start_kill();
        }
        for managed in &mut self.children {
            let _ = tokio::time::timeout(Duration::from_secs(5), managed.child.wait()).await;
        }
        self.children.clear();
        self.temp_files.clear();
    }

    async fn kill_child(&mut self, label: &str) -> anyhow::Result<()> {
        let position = self
            .children
            .iter()
            .position(|managed| managed.label == label)
            .ok_or_else(|| anyhow::anyhow!("missing child process {label}"))?;
        let mut managed = self.children.swap_remove(position);
        managed.child.start_kill()?;
        let _ = tokio::time::timeout(Duration::from_secs(5), managed.child.wait()).await;
        Ok(())
    }

    async fn reap_child(&mut self, label: &str) -> anyhow::Result<()> {
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

    async fn reap_children(&mut self, labels: &[&str]) -> anyhow::Result<()> {
        for label in labels {
            self.reap_child(label).await?;
        }
        Ok(())
    }

    #[cfg(unix)]
    async fn terminate_child(&mut self, label: &str) -> anyhow::Result<()> {
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

        let _ = tokio::time::timeout(Duration::from_secs(10), managed.child.wait()).await;
        Ok(())
    }
}

impl Drop for FullStackHarness {
    fn drop(&mut self) {
        for managed in &mut self.children {
            let _ = managed.child.start_kill();
        }
    }
}

fn temp_run_config(contents: &str) -> NamedTempFile {
    let file = NamedTempFile::new().expect("create temp config");
    std::fs::write(file.path(), contents).expect("write temp config");
    file
}

fn temp_run_add_config(contents: &str) -> NamedTempFile {
    let mut merged = contents.trim_end().to_string();
    if !contents.contains("evaluator_runner_params.min_tick_time_ms")
        && !contents.contains("[evaluator_runner_params]")
    {
        merged.push_str("\n\nevaluator_runner_params.min_tick_time_ms = 50\n");
    }
    if !contents.contains("sampler_aggregator_runner_params.min_tick_time_ms")
        && !contents.contains("[sampler_aggregator_runner_params]")
    {
        merged.push_str("\nsampler_aggregator_runner_params.min_tick_time_ms = 10\n");
    }
    temp_run_config(&merged)
}

async fn run_havana_training_then_inference(
    harness: &mut FullStackHarness,
    run_name: &str,
    pause_mid_training: bool,
) -> anyhow::Result<(JsonValue, JsonValue)> {
    let training_samples = 256usize;
    let inference_samples = 64usize;
    let config = temp_run_add_config(&format!(
        r#"
name = "{run_name}"

[evaluator]
kind = "unit"
continuous_dims = 2
discrete_dims = 0

[[task_queue]]
name = "train-a"
kind = "sample"
stop_condition = {{ max_samples = {training_samples} }}
accumulator = {{ config = "scalar" }}
sampler_aggregator = {{ config = {{ kind = "havana_training", seed = 0, bins = 8, samples_for_update = 8, initial_training_rate = 0.1, final_training_rate = 0.01 }} }}
"#,
    ));

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 = sqlx::query_scalar("SELECT id FROM runs WHERE name = $1")
        .bind(run_name)
        .fetch_one(&harness.pool)
        .await?;

    harness
        .cli()
        .args(["node", "assign", "w-2", "evaluator", run_name])
        .assert()
        .success();
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

    harness
        .cli()
        .args(["node", "assign", "w-1", "sampler-aggregator", run_name])
        .assert()
        .success();

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

        harness
            .cli()
            .args(["node", "assign", "w-2", "evaluator", run_name])
            .assert()
            .success();
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
        harness
            .cli()
            .args(["node", "assign", "w-1", "sampler-aggregator", run_name])
            .assert()
            .success();

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

    let inference_task = temp_run_config(&format!(
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
            "add",
            &run_id.to_string(),
            inference_task.path().to_str().expect("task file path"),
        ])
        .assert()
        .success();

    harness
        .cli()
        .args(["node", "assign", "w-2", "evaluator", run_name])
        .assert()
        .success();
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
    harness
        .cli()
        .args(["node", "assign", "w-1", "sampler-aggregator", run_name])
        .assert()
        .success();

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

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_alternating_havana_e2e() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    // Initial run with tasks 1..4:
    // 1: havana_training
    // 2: havana_inference
    // 3: naive_monte_carlo
    // 4: image
    let config = temp_run_add_config(
        r#"
name = "havana-alt-e2e"

[evaluator]
kind = "unit"
continuous_dims = 2
discrete_dims = 0

[[task_queue]]
name = "train-a"
kind = "sample"
stop_condition = { max_samples = 128 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "havana_training", seed = 0, bins = 8, samples_for_update = 8 } }

[[task_queue]]
name = "infer-a"
kind = "sample"
stop_condition = { max_samples = 128 }
sampler_aggregator = { config = { kind = "havana_inference" } }

[[task_queue]]
name = "naive-a"
kind = "sample"
stop_condition = { max_samples = 32 }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }

[[task_queue]]
kind = "image"
accumulator = "scalar"
[task_queue.geometry]
offset = [0.0, 0.0]
u_vector = [1.0, 0.0]
v_vector = [0.0, 1.0]
[task_queue.geometry.u_linspace]
start = -1.0
stop = 1.0
count = 8
[task_queue.geometry.v_linspace]
start = -1.0
stop = 1.0
count = 8
"#,
    );

    // Create the run
    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 = sqlx::query_scalar("SELECT id FROM runs WHERE name = 'havana-alt-e2e'")
        .fetch_one(&harness.pool)
        .await?;

    // Start nodes and assign roles
    harness.start_nodes(&["w-1", "w-2"]).await?;

    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-1",
            "sampler-aggregator",
            "havana-alt-e2e",
        ])
        .assert()
        .success();
    harness
        .cli()
        .args(["node", "assign", "w-2", "evaluator", "havana-alt-e2e"])
        .assert()
        .success();

    // Wait for the first four tasks to complete (sequence_nr 1..4)
    harness
        .wait_for("first 4 tasks complete", Duration::from_secs(60), || {
            let pool = harness.pool.clone();
            async move {
                let completed: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM run_tasks WHERE run_id = $1 AND state = 'completed' AND sequence_nr >= 1 AND sequence_nr <= 4",
                )
                .bind(run_id)
                .fetch_one(&pool)
                .await?;
                Ok(completed == 4)
            }
        })
        .await?;

    // Now append task 5 and 6:
    // 5: resumes directly from task "infer-a"
    // 6: havana_inference (uses most recent compatible training/inference snapshot by default)
    let tasks_toml = r#"
[[task_queue]]
kind = "sample"
stop_condition = { max_samples = 128 }
sampler_aggregator = { from_name = "infer-a" }
accumulator = { from_name = "infer-a" }

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = 128 }
sampler_aggregator = { config = { kind = "havana_inference" } }
"#
    .to_string();

    let task_file = temp_run_config(&tasks_toml);

    harness
        .cli()
        .args([
            "run",
            "task",
            "add",
            &run_id.to_string(),
            task_file.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // Reassign nodes so the newly appended tasks will be picked up by workers.
    // Use auto-assign to let the system pick appropriate nodes.
    harness
        .cli()
        .args(["auto-assign", &run_id.to_string()])
        .assert()
        .success();

    // Wait for all 6 tasks to complete
    harness
        .wait_for("all tasks complete", Duration::from_secs(120), || {
            let pool = harness.pool.clone();
            async move {
                let completed: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM run_tasks WHERE run_id = $1 AND state = 'completed'",
                )
                .bind(run_id)
                .fetch_one(&pool)
                .await?;
                Ok(completed == 6)
            }
        })
        .await?;

    // Verify task 5 has the expected named source reference
    let t5_sampler_source: Option<String> = sqlx::query_scalar(
        "SELECT task->'sampler_aggregator'->>'from_name' FROM run_tasks WHERE run_id = $1 AND sequence_nr = 5",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(t5_sampler_source.as_deref(), Some("infer-a"));

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_symbolica_havana_pdf_two_bumps_e2e() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config = temp_run_add_config(
        r#"
name = "symbolica-havana-pdf-1d2d-e2e"

[evaluator]
kind = "symbolica"
expr = "1/((x-1/4)^2+(y-1/4)^2+1/40) + 1/((x-3/4)^2+(y-3/4)^2+1/40) + z"
args = ["x", "y", "z"]

[evaluator_runner_params]
performance_snapshot_interval_ms = 2000
min_tick_time_ms = 50
db_pool_size = 2

[sampler_aggregator_runner_params]
performance_snapshot_interval_ms = 2000
min_tick_time_ms = 50
frontend_sync_interval_ms = 2000
db_pool_size = 10

[sampler_aggregator_runner_params.queue]
queue_buffer = 1.0
target_batch_eval_ms = 500.0
batch_size_deadband_ratio = 0.15
batch_size_cooldown_ticks = 3
pending_refill_low_ratio = 0.85
pending_refill_high_ratio = 1.15
max_batch_size = 100000
local_pending_buffer_multiplier = 0.5
max_queue_size = 200
max_batches_per_tick = 100
max_insert_bundle_size = 5
max_concurrent_insert_tasks = 8
completed_batch_fetch_limit = 100
max_batch_retries = 3

[[task_queue]]
name = "accumulator"
kind = "set_accumulator"
accumulator = "scalar"

[[task_queue]]
name = "havana-train"
kind = "sample"
[task_queue.stop_condition]
max_samples = 200000
[task_queue.sampler_aggregator.config]
kind = "havana_training"
seed = 0
bins = 64
samples_for_update = 16384
initial_training_rate = 0.1
final_training_rate = 0.001

[[task_queue]]
name = "pdf-2d"
kind = "pdf_adaptation_image"
batch_transforms = []

[task_queue.geometry]
offset = [0.0, 0.0, 0.0]
u_vector = [1.0, 0.0, 0.0]
v_vector = [0.0, 1.0, 0.0]
discrete = []

[task_queue.geometry.u_linspace]
start = 0.0
stop = 1.0
count = 128

[task_queue.geometry.v_linspace]
start = 0.0
stop = 1.0
count = 128
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 =
        sqlx::query_scalar("SELECT id FROM runs WHERE name = 'symbolica-havana-pdf-1d2d-e2e'")
            .fetch_one(&harness.pool)
            .await?;

    harness.start_nodes(&["w-1", "w-2"]).await?;
    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-1",
            "sampler-aggregator",
            "symbolica-havana-pdf-1d2d-e2e",
        ])
        .assert()
        .success();
    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-2",
            "evaluator",
            "symbolica-havana-pdf-1d2d-e2e",
        ])
        .assert()
        .success();

    let pdf_task_id: i64 = sqlx::query_scalar(
        "SELECT id FROM run_tasks WHERE run_id = $1 AND name = 'pdf-2d' LIMIT 1",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;

    let expected_width = 128usize;
    let expected_height = 128usize;
    let expected_points = expected_width * expected_height;
    let mut seen_batch_ids = HashSet::<i64>::new();
    let mut seen_grid_points = HashSet::<(usize, usize)>::new();
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut min_z = f64::INFINITY;
    let mut max_z = f64::NEG_INFINITY;
    let mut observed_points = 0usize;
    let mut last_seen_batch_id = 0_i64;
    let sampling_deadline = Instant::now() + Duration::from_secs(180);
    while seen_grid_points.len() < expected_points {
        let rows: Vec<(i64, Vec<u8>)> = sqlx::query_as(
            r#"
            SELECT b.id, bi.latent_batch
            FROM batch_inputs bi
            JOIN batches b ON b.id = bi.batch_id
            WHERE b.run_id = $1 AND b.task_id = $2 AND b.id > $3
            ORDER BY b.id ASC
            "#,
        )
        .bind(run_id)
        .bind(pdf_task_id)
        .bind(last_seen_batch_id)
        .fetch_all(&harness.pool)
        .await?;

        for (batch_id, payload) in rows {
            last_seen_batch_id = last_seen_batch_id.max(batch_id);
            if !seen_batch_ids.insert(batch_id) {
                continue;
            }
            let latent = LatentBatch::from_bytes(&payload)?;
            let batch = latent.payload.as_batch()?;
            for point in batch.points() {
                assert_eq!(
                    point.continuous.len(),
                    3,
                    "expected 3 continuous dimensions for symbolica args x,y,z"
                );
                let x = point.continuous[0];
                let y = point.continuous[1];
                let z = point.continuous[2];
                assert!(x.is_finite() && y.is_finite() && z.is_finite());
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
                min_z = min_z.min(z);
                max_z = max_z.max(z);
                observed_points += 1;

                let u = (x * (expected_width - 1) as f64).round();
                let v = (y * (expected_height - 1) as f64).round();
                assert!(
                    (u - x * (expected_width - 1) as f64).abs() <= 1e-9,
                    "x={x} is off the 128-point linspace grid"
                );
                assert!(
                    (v - y * (expected_height - 1) as f64).abs() <= 1e-9,
                    "y={y} is off the 128-point linspace grid"
                );
                let u = u as usize;
                let v = v as usize;
                assert!(u < expected_width && v < expected_height);
                seen_grid_points.insert((u, v));
            }
        }

        if Instant::now() >= sampling_deadline {
            anyhow::bail!(
                "timed out while validating evaluator input points: observed_unique={} expected={} observed_points={} observed_batches={}",
                seen_grid_points.len(),
                expected_points,
                observed_points,
                seen_batch_ids.len()
            );
        }
        sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        seen_grid_points.len(),
        expected_points,
        "did not observe every raster point"
    );
    assert!(observed_points > 0, "expected at least one evaluated point");
    assert!(
        min_x >= -1e-12 && max_x <= 1.0 + 1e-12,
        "x outside [0,1]: min={min_x}, max={max_x}"
    );
    assert!(
        min_y >= -1e-12 && max_y <= 1.0 + 1e-12,
        "y outside [0,1]: min={min_y}, max={max_y}"
    );
    assert!(
        min_z >= -1e-12 && max_z <= 1e-12,
        "z should be fixed at 0: min={min_z}, max={max_z}"
    );

    harness
        .wait_for("all tasks complete", Duration::from_secs(180), || {
            let pool = harness.pool.clone();
            async move {
                let completed: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM run_tasks WHERE run_id = $1 AND state = 'completed'",
                )
                .bind(run_id)
                .fetch_one(&pool)
                .await?;
                Ok(completed == 3)
            }
        })
        .await?;

    let persisted = harness
        .latest_task_persisted_observable(run_id, "pdf-2d")
        .await?;
    let output: PdfAdaptationImagePersistedOutput = serde_json::from_value(persisted)?;
    assert!(
        output.global_pdf_norm.is_finite() && output.global_pdf_norm > 0.0,
        "expected positive global pdf norm, got {}",
        output.global_pdf_norm
    );
    assert!(
        output
            .global_abs_integrand_norm
            .is_some_and(|value| value.is_finite() && value > 0.0),
        "expected positive global integrand norm in persisted output"
    );

    let width = expected_width;
    let height = expected_height;
    assert_eq!(output.abs_integrand_values.len(), width * height);

    let value_at = |u: usize, v: usize| -> f64 {
        output.abs_integrand_values[v * width + u]
            .filter(|value| value.is_finite())
            .unwrap_or(f64::NEG_INFINITY)
    };

    let mut local_maxima = Vec::<(usize, usize, f64)>::new();
    for v in 0..height {
        for u in 0..width {
            let center = value_at(u, v);
            if !center.is_finite() {
                continue;
            }
            let mut is_local_max = true;
            for dv in -1_i32..=1 {
                for du in -1_i32..=1 {
                    if du == 0 && dv == 0 {
                        continue;
                    }
                    let nu = u as i32 + du;
                    let nv = v as i32 + dv;
                    if nu < 0 || nv < 0 || nu >= width as i32 || nv >= height as i32 {
                        continue;
                    }
                    if value_at(nu as usize, nv as usize) > center {
                        is_local_max = false;
                        break;
                    }
                }
                if !is_local_max {
                    break;
                }
            }
            if is_local_max {
                local_maxima.push((u, v, center));
            }
        }
    }

    local_maxima.sort_by(|a, b| b.2.partial_cmp(&a.2).expect("finite maxima values"));
    assert!(
        local_maxima.len() >= 2,
        "expected at least two local maxima, got {}",
        local_maxima.len()
    );

    let first = local_maxima[0];
    let second = local_maxima
        .iter()
        .copied()
        .find(|(u, v, _)| {
            let du = (*u as isize - first.0 as isize).unsigned_abs();
            let dv = (*v as isize - first.1 as isize).unsigned_abs();
            du + dv >= 16
        })
        .ok_or_else(|| anyhow::anyhow!("failed to find a second distinct peak"))?;

    let to_param = |index: usize| -> f64 { index as f64 / (width - 1) as f64 };
    let (t1, s1) = (to_param(first.0), to_param(first.1));
    let (t2, s2) = (to_param(second.0), to_param(second.1));

    let near = |a: f64, b: f64| (a - b).abs() <= 0.08;
    let first_match = near(t1, 0.25) && near(s1, 0.25);
    let second_match = near(t2, 0.75) && near(s2, 0.75);
    let swapped_match = near(t1, 0.75) && near(s1, 0.75) && near(t2, 0.25) && near(s2, 0.25);

    assert!(
        (first_match && second_match) || swapped_match,
        "peak positions mismatch: first=({t1:.4},{s1:.4}) second=({t2:.4},{s2:.4})"
    );

    harness.stop_children().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_havana_pause_resume_matches_direct_baseline() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    let training_samples = 256usize;
    let havana_params = HavanaSamplerParams {
        seed: 0,
        bins: 8,
        samples_for_update: 8,
        initial_training_rate: 0.1,
        final_training_rate: 0.01,
    };
    harness.start_nodes(&["w-1", "w-2"]).await?;

    let (uninterrupted_training_grid, uninterrupted_inference_grid) =
        run_havana_training_then_inference(
            &mut harness,
            "havana-uninterrupted-determinism-e2e",
            false,
        )
        .await?;
    let (paused_training_grid, paused_inference_grid) =
        run_havana_training_then_inference(&mut harness, "havana-paused-determinism-e2e", true)
            .await?;
    let direct_grid = serde_json::to_value(direct_train_havana_grid(
        &Domain::continuous(2),
        &havana_params,
        training_samples,
    ))?;

    assert_eq!(uninterrupted_training_grid, direct_grid);
    assert_eq!(paused_training_grid, direct_grid);
    assert_eq!(uninterrupted_inference_grid, uninterrupted_training_grid);
    assert_eq!(paused_inference_grid, paused_training_grid);
    assert_eq!(paused_training_grid, uninterrupted_training_grid);
    assert_eq!(paused_inference_grid, uninterrupted_inference_grid);

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege, scalar-sin .venv, nix, and python+numpy"]
async fn full_stack_cli_python_scalar_venv_e2e() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness.start_nodes(&["w-1", "w-2"]).await?;

    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let evaluator_dir = manifest_dir.join("process_api/examples/python_scalar_sin");
    let evaluator_python = evaluator_dir.join(".venv/bin/python");
    let process_api_python = manifest_dir.join("process_api/python/src");
    let sampler_src = manifest_dir.join("process_api/examples/python_sampler_symbolica_havana/src");
    let sampler_pythonpath = format!("{}:{}", process_api_python.display(), sampler_src.display());
    let sampler_flake_ref = format!(
        "path:{}#runtime",
        manifest_dir
            .join("process_api/examples/python_sampler_symbolica_havana")
            .display()
    );
    let config = temp_run_add_config(&format!(
        r#"
name = "python-scalar-venv-e2e"

[evaluator]
kind = "process_evaluator"
command = ["env", "PYTHONPATH={}", "{}", "-u", "-m", "run_evaluator"]
cwd = "{}"
domain = {{ rectangular = {{ discrete_cardinalities = [2, 3], continuous_dims = 2 }} }}
args = {{ scale = 1.0, bias = 0.0, freq_u = 2.0, freq_v = 1.25 }}

[[task_queue]]
name = "accumulator"
kind = "set_accumulator"

[task_queue.accumulator]
kind = "vector"
components = ["value"]
training_projection = {{ kind = "component", name = "value" }}

[task_queue.accumulator.discrete_projections]
[[task_queue.accumulator.discrete_projections.items]]
name = "spin"
dims = [0]
fixed_dims = {{}}

[[task_queue.accumulator.discrete_projections.items]]
name = "channel_for_spin_0"
dims = [1]
fixed_dims = {{ "0" = 0 }}

[[task_queue]]
name = "sample-a"
kind = "sample"
stop_condition = {{ max_samples = 64 }}
sampler_aggregator = {{ config = {{ kind = "process_sampler", command = ["nix", "shell", "{sampler_flake_ref}", "-c", "env", "PYTHONPATH={sampler_pythonpath}", "python", "-u", "-m", "run_sampler"], requires_training_values = true, args = {{ seed = 0, bins = 8, samples_for_update = 8, stop_training_after_n_samples = 64, initial_training_rate = 0.1, final_training_rate = 0.01 }} }} }}
"#,
        process_api_python.display(),
        evaluator_python.display(),
        evaluator_dir.join("src").display(),
    ));

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 =
        sqlx::query_scalar("SELECT id FROM runs WHERE name = 'python-scalar-venv-e2e'")
            .fetch_one(&harness.pool)
            .await?;

    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-1",
            "sampler-aggregator",
            "python-scalar-venv-e2e",
        ])
        .assert()
        .success();
    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-2",
            "evaluator",
            "python-scalar-venv-e2e",
        ])
        .assert()
        .success();

    harness
        .wait_for(
            "python scalar venv task completes",
            Duration::from_secs(120),
            || async {
                let (state, failure_reason): (String, Option<String>) = sqlx::query_as(
                    "SELECT state, failure_reason FROM run_tasks WHERE run_id = $1 AND name = 'sample-a'",
                )
                .bind(run_id)
                .fetch_one(&harness.pool)
                .await?;
                anyhow::ensure!(
                    state != "failed",
                    "sample-a failed: {}",
                    failure_reason.unwrap_or_else(|| "no failure_reason".to_string())
                );
                Ok(state == "completed")
            },
        )
        .await?;

    harness
        .wait_for(
            "python symbolica havana sampler checkpoint is persisted",
            Duration::from_secs(30),
            || async { Ok(harness.run_sampler_checkpoint(run_id).await?.is_some()) },
        )
        .await?;

    let completed_samples: i64 =
        sqlx::query_scalar("SELECT nr_completed_samples FROM runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(&harness.pool)
            .await?;
    assert!(completed_samples >= 64);

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres, nix, the integrations/madnis runtime, and the bundled GammaLoop state"]
async fn full_stack_cli_gammaloop_madnis_metadata_and_batch_fuzz_e2e() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let process_api_python = manifest_dir.join("process_api/python/src");
    let madnis_src = manifest_dir.join("integrations/madnis/src");
    let gammaloop_state = manifest_dir.join("resources/states/epem_a_ttxh/LO/state");
    let madnis_pythonpath = format!("{}:{}", process_api_python.display(), madnis_src.display());
    let madnis_flake_ref = format!(
        "path:{}#runtime",
        manifest_dir.join("integrations/madnis").display()
    );
    let run_name = format!("gammaloop-madnis-metadata-fuzz-{}", unique_suffix());
    let cases = [(1_usize, 1_usize, 2_usize), (2, 1, 3), (3, 2, 4)];

    let madnis_runtime_status = std::process::Command::new("nix")
        .args([
            "shell",
            &madnis_flake_ref,
            "-c",
            "env",
            &format!("PYTHONPATH={madnis_pythonpath}"),
            "python",
            "-c",
            "import run_sampler",
        ])
        .current_dir(manifest_dir.join("resources"))
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    anyhow::ensure!(
        madnis_runtime_status.success(),
        "failed to preflight MadNIS runtime"
    );

    harness.start_nodes(&["w-1", "w-2"]).await?;

    let mut task_toml = String::new();
    for (case_idx, (queue_max_batch_size, madnis_max_batch_size, samples_for_update)) in
        cases.iter().copied().enumerate()
    {
        task_toml.push_str(&format!(
            r#"

[[task_queue]]
name = "madnis-case-{case_idx}"
kind = "sample"
stop_condition = {{ max_samples = {samples_for_update} }}
accumulator = "latest"
sampler_aggregator = {{ config = {{ kind = "process_sampler", command = ["nix", "shell", "{madnis_flake_ref}", "-c", "env", "PYTHONPATH={madnis_pythonpath}", "python", "-u", "-m", "run_sampler"], requires_training_values = true, args = {{ seed = {case_idx}, training_steps = 1, training_batch_size = {samples_for_update}, max_batch_size = {madnis_max_batch_size}, use_gpu = false, learning_rate = 0.001, use_scheduler = false, discrete_model = "made", discrete_dims_position = "first", flow_config = {{ layers = 1, units = 8, bins = 4 }}, made_config = {{ layers = 1, nodes_per_feature = 8 }} }} }} }}

[task_queue.queue_tuning]
max_batch_size = {queue_max_batch_size}
target_batch_eval_ms = 1.0
"#
        ));
    }

    let config = temp_run_add_config(&format!(
        r#"
name = "{run_name}"

[evaluator]
kind = "gammaloop"
state_folder = "{}"
integrand_name = "LO"
training_projection = "abs"

[evaluator.preprocessing]
read_only = true
commands = [
  "set model MT=173.0",
  "set model WT=0.0",
  "set model ymt=173.0",
  "set model aS=0.118",
  "set model aEWM1=132.507",
  "set model Gf=1.166390e-05",
  "set model MZ=91.188",
]

[sampler_aggregator_runner_params]
performance_snapshot_interval_ms = 100
min_tick_time_ms = 10
frontend_sync_interval_ms = 100
db_pool_size = 2

[evaluator_runner_params]
performance_snapshot_interval_ms = 100
min_tick_time_ms = 50
db_pool_size = 1

[[task_queue]]
name = "accumulator"
kind = "set_accumulator"

[task_queue.accumulator]
kind = "vector"
components = ["real", "imag"]
training_projection = {{ kind = "component", name = "real" }}

{task_toml}
"#,
        gammaloop_state.display()
    ));

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 = sqlx::query_scalar("SELECT id FROM runs WHERE name = $1")
        .bind(&run_name)
        .fetch_one(&harness.pool)
        .await?;

    harness
        .cli()
        .args(["node", "assign", "w-1", "sampler-aggregator", &run_name])
        .assert()
        .success();
    harness
        .cli()
        .args(["node", "assign", "w-2", "evaluator", &run_name])
        .assert()
        .success();

    for case_idx in 0..cases.len() {
        let task_name = format!("madnis-case-{case_idx}");
        harness
            .wait_for(
                format!("GammaLoop/MadNIS task {task_name} completes"),
                Duration::from_secs(300),
                || {
                    let pool = harness.pool.clone();
                    let task_name = task_name.clone();
                    async move {
                        let (state, failure_reason): (String, Option<String>) = sqlx::query_as(
                            "SELECT state, failure_reason FROM run_tasks WHERE run_id = $1 AND name = $2",
                        )
                        .bind(run_id)
                        .bind(&task_name)
                        .fetch_one(&pool)
                        .await?;
                        anyhow::ensure!(
                            state != "failed",
                            "{task_name} failed: {}",
                            failure_reason.unwrap_or_else(|| "no failure_reason".to_string())
                        );
                        Ok(state == "completed")
                    }
                },
            )
            .await?;
    }

    harness
        .wait_for(
            "MadNIS diagnostics include GammaLoop evaluator metadata",
            Duration::from_secs(30),
            || {
                let pool = harness.pool.clone();
                async move {
                    let diagnostics: Option<JsonValue> = sqlx::query_scalar(
                        r#"
                        SELECT engine_diagnostics
                        FROM sampler_aggregator_performance_latest
                        WHERE run_id = $1 AND worker_id = 'w-1'
                        "#,
                    )
                    .bind(run_id)
                    .fetch_optional(&pool)
                    .await?;
                    let Some(diagnostics) = diagnostics else {
                        return Ok(false);
                    };
                    let metadata = &diagnostics["gammaloop_metadata"];
                    Ok(metadata["state_folder"].as_str().is_some_and(|path| {
                        path.ends_with("resources/states/epem_a_ttxh/LO/state")
                    }) && metadata["process_id"].as_u64().is_some()
                        && metadata["integrand_name"].as_str() == Some("LO")
                        && metadata["coordinate_space"].as_str() == Some("x_space")
                        && diagnostics["produced_batches"].as_u64().unwrap_or(0) > 0)
                }
            },
        )
        .await?;

    let completed_samples: i64 =
        sqlx::query_scalar("SELECT nr_completed_samples FROM runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(&harness.pool)
            .await?;
    let expected_samples: i64 = cases
        .iter()
        .map(|(_, _, samples_for_update)| *samples_for_update as i64)
        .sum();
    assert!(completed_samples >= expected_samples);

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_task_level_evaluator_switch_e2e() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness.start_nodes(&["w-1", "w-2"]).await?;

    let config = temp_run_add_config(
        r#"
name = "task-level-evaluator-switch-e2e"

[evaluator]
kind = "symbolica"
expr = "1"
args = ["x"]

[[task_queue]]
name = "accumulator-a"
kind = "set_accumulator"
accumulator = "scalar"

[[task_queue]]
name = "sample-a"
kind = "sample"
stop_condition = { max_samples = 32 }
evaluator = { config = { kind = "symbolica", expr = "1", args = ["x"] } }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
accumulator = "latest"

[[task_queue]]
name = "accumulator-b"
kind = "set_accumulator"
accumulator = "scalar"

[[task_queue]]
name = "sample-b"
kind = "sample"
stop_condition = { max_samples = 32 }
evaluator = { config = { kind = "symbolica", expr = "2", args = ["x"] } }
sampler_aggregator = "latest"
accumulator = "latest"
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 =
        sqlx::query_scalar("SELECT id FROM runs WHERE name = 'task-level-evaluator-switch-e2e'")
            .fetch_one(&harness.pool)
            .await?;

    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-1",
            "sampler-aggregator",
            "task-level-evaluator-switch-e2e",
        ])
        .assert()
        .success();
    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-2",
            "evaluator",
            "task-level-evaluator-switch-e2e",
        ])
        .assert()
        .success();

    harness
        .wait_for(
            "task-level evaluator tasks complete",
            Duration::from_secs(60),
            || async {
                let completed: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM run_tasks WHERE run_id = $1 AND state = 'completed'",
                )
                .bind(run_id)
                .fetch_one(&harness.pool)
                .await?;
                Ok(completed == 4)
            },
        )
        .await?;

    let rows: Vec<(String, JsonValue, JsonValue)> = sqlx::query_as(
        r#"
        SELECT DISTINCT ON (t.name) t.name, s.evaluator, s.observable_state
        FROM run_stage_snapshots s
        JOIN run_tasks t ON t.id = s.task_id
        WHERE s.run_id = $1
          AND t.name IN ('sample-a', 'sample-b')
          AND s.queue_empty = TRUE
        ORDER BY t.name, s.id DESC
        "#,
    )
    .bind(run_id)
    .fetch_all(&harness.pool)
    .await?;

    assert_eq!(rows.len(), 2);
    let mean = |state: &JsonValue| -> f64 {
        let sum = state
            .get("sum_weighted_value")
            .and_then(JsonValue::as_f64)
            .expect("sum_weighted_value");
        let count = state
            .get("count")
            .and_then(JsonValue::as_i64)
            .expect("count") as f64;
        sum / count
    };
    assert_eq!(rows[0].0, "sample-a");
    assert_eq!(rows[0].1.get("expr").and_then(JsonValue::as_str), Some("1"));
    assert!((mean(&rows[0].2) - 1.0).abs() < 1e-12);
    assert_eq!(rows[1].0, "sample-b");
    assert_eq!(rows[1].1.get("expr").and_then(JsonValue::as_str), Some("2"));
    assert!((mean(&rows[1].2) - 2.0).abs() < 1e-12);

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege, apptainer, and a working unprivileged Apptainer build setup"]
async fn full_stack_cli_rust_apptainer_process_evaluator_e2e() -> anyhow::Result<()> {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let example_dir = manifest_dir.join("process_api/examples/rust_breit_wigner_evaluator");
    let image_path = example_dir.join("runtime.sif");
    let _ = std::fs::remove_file(&image_path);
    let _image_cleanup = RemoveFileOnDrop(image_path.clone());

    let build_output = std::process::Command::new("apptainer")
        .arg("build")
        .arg("--force")
        .arg(&image_path)
        .arg("apptainer.def")
        .current_dir(&example_dir)
        .output()?;
    anyhow::ensure!(
        build_output.status.success(),
        "apptainer build failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&build_output.stdout),
        String::from_utf8_lossy(&build_output.stderr)
    );

    let mut harness = FullStackHarness::new().await?;
    harness.start_nodes(&["w-1", "w-2"]).await?;

    let config = temp_run_add_config(&format!(
        r#"
name = "rust-apptainer-process-evaluator-e2e"

[evaluator]
kind = "process_evaluator"
command = ["apptainer", "exec", "{}", "breit-wigner-worker"]
domain = {{ discrete = {{ axis_label = "d0", branches = [
  {{ index = 0, domain = {{ continuous = {{ dims = 3 }} }} }},
  {{ index = 1, domain = {{ discrete = {{ axis_label = "d1", branches = [
    {{ index = 0, domain = {{ continuous = {{ dims = 1 }} }} }},
    {{ index = 1, domain = {{ rectangular = {{ discrete_cardinalities = [5], continuous_dims = 5 }} }} }},
  ] }} }} }},
] }} }}
args = {{ masses = [0.25, 0.50, 0.75], widths = [0.04, 0.06, 0.05], channel_weights = [1.0, 0.7, 1.3] }}

[[task_queue]]
name = "accumulator"
kind = "set_accumulator"

[task_queue.accumulator]
kind = "vector"
components = ["value"]
training_projection = {{ kind = "component", name = "value" }}

[[task_queue]]
name = "sample-a"
kind = "sample"
stop_condition = {{ max_samples = 64 }}
sampler_aggregator = {{ config = {{ kind = "naive_monte_carlo" }} }}
"#,
        "../process_api/examples/rust_breit_wigner_evaluator/runtime.sif"
    ));

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 = sqlx::query_scalar(
        "SELECT id FROM runs WHERE name = 'rust-apptainer-process-evaluator-e2e'",
    )
    .fetch_one(&harness.pool)
    .await?;

    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-1",
            "sampler-aggregator",
            "rust-apptainer-process-evaluator-e2e",
        ])
        .assert()
        .success();
    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-2",
            "evaluator",
            "rust-apptainer-process-evaluator-e2e",
        ])
        .assert()
        .success();

    harness
        .wait_for(
            "rust apptainer process evaluator task completes",
            Duration::from_secs(120),
            || async {
                let (state, failure_reason): (String, Option<String>) = sqlx::query_as(
                    "SELECT state, failure_reason FROM run_tasks WHERE run_id = $1 AND name = 'sample-a'",
                )
                .bind(run_id)
                .fetch_one(&harness.pool)
                .await?;
                anyhow::ensure!(
                    state != "failed",
                    "sample-a failed: {}",
                    failure_reason.unwrap_or_else(|| "no failure_reason".to_string())
                );
                Ok(state == "completed")
            },
        )
        .await?;

    let completed_samples: i64 =
        sqlx::query_scalar("SELECT nr_completed_samples FROM runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(&harness.pool)
            .await?;
    assert!(completed_samples >= 64);

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

fn temp_server_config(
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
    let file = NamedTempFile::new().expect("create temp server config");
    std::fs::write(file.path(), contents).expect("write temp server config");
    file
}

fn temp_deploy_server_config(
    frontend_build_dir: &std::path::Path,
    server_config: &std::path::Path,
    frontend_port: u16,
) -> NamedTempFile {
    let contents = std::fs::read_to_string(server_config).expect("read temp server config");
    let contents = format!(
        "{contents}\n[frontend]\nbuild_dir = {:?}\nhost = \"127.0.0.1\"\nport = {frontend_port}\nadvertise_hosts = [\"localhost\"]\naccess_log = false\n\n[database]\nensure_started = false\n\n[cleanup]\nsampler_drain_timeout_seconds = 5\nnode_stop_timeout_seconds = 5\npoll_interval_ms = 100\n",
        frontend_build_dir,
    );
    let file = NamedTempFile::new().expect("create temp deploy server config");
    std::fs::write(file.path(), contents).expect("write temp deploy server config");
    file
}

fn temp_frontend_build() -> TempDir {
    let dir = tempfile::tempdir().expect("create temp frontend build");
    std::fs::write(
        dir.path().join("index.html"),
        "<!doctype html><html><body>gammaboard e2e</body></html>",
    )
    .expect("write index.html");
    dir
}

fn temp_cli_config(database_url: &str, persist_runtime_logs: bool) -> NamedTempFile {
    let contents = format!(
        "[database]\nurl = {database_url:?}\n\n[tracing]\npersist_runtime_logs = {persist_runtime_logs}\ndb_gammaboard_level = \"info\"\ndb_external_level = \"warn\"\n\n[local_postgres]\ndata_dir = \".postgres\"\nsocket_dir = \"db/socket\"\nlog_file = \".postgres/logfile\"\nmax_connections = 512\n"
    );
    let file = NamedTempFile::new().expect("create temp cli config");
    std::fs::write(file.path(), contents).expect("write temp cli config");
    file
}

async fn http_get(base_url: &str, path: &str) -> anyhow::Result<String> {
    let url = Url::parse(base_url)?.join(path)?;
    let response = reqwest::get(url).await?;
    let body = response.error_for_status()?.text().await?;
    Ok(body)
}

async fn http_get_with_cookie(base_url: &str, path: &str, cookie: &str) -> anyhow::Result<String> {
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

async fn http_post_json(
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

fn hash_password_for_tests(password: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .expect("argon2 hash")
        .to_string()
}

fn build_direct_havana_grid(domain: &Domain, params: &HavanaSamplerParams) -> Grid<f64> {
    const DEFAULT_DISCRETE_MAX_PROB_RATIO: f64 = 30.0;

    match domain {
        Domain::Continuous { dims } => Grid::Continuous(ContinuousGrid::new(
            *dims,
            params.bins,
            params.samples_for_update,
            None,
            false,
        )),
        Domain::Rectangular {
            discrete_cardinalities,
            continuous_dims,
        } => build_direct_rectangular_havana_grid(*continuous_dims, discrete_cardinalities, params),
        Domain::Discrete { branches, .. } => {
            let bins = branches
                .iter()
                .map(|branch| Some(build_direct_havana_grid(branch.domain.as_ref(), params)))
                .collect();
            Grid::Discrete(DiscreteGrid::new(
                bins,
                DEFAULT_DISCRETE_MAX_PROB_RATIO,
                false,
            ))
        }
    }
}

fn build_direct_rectangular_havana_grid(
    continuous_dims: usize,
    discrete_cardinalities: &[usize],
    params: &HavanaSamplerParams,
) -> Grid<f64> {
    const DEFAULT_DISCRETE_MAX_PROB_RATIO: f64 = 30.0;
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
        return Grid::Discrete(DiscreteGrid::new(
            bins,
            DEFAULT_DISCRETE_MAX_PROB_RATIO,
            false,
        ));
    }
    Grid::Continuous(ContinuousGrid::new(
        continuous_dims,
        params.bins,
        params.samples_for_update,
        None,
        false,
    ))
}

fn direct_unit_training_value(_sample: &Sample<f64>) -> f64 {
    1.0
}

fn direct_havana_training_rate(
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

fn direct_train_havana_grid(
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

async fn wait_for_task_failed_and_run_unassigned(
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
            let w1 = harness.node_state("w-1").await?;
            let w2 = harness.node_state("w-2").await?;
            Ok(state == "failed"
                && failure_reason.is_some()
                && w1.0.is_none()
                && w1.1.is_none()
                && w1.2.is_none()
                && w1.3.is_none()
                && w2.0.is_none()
                && w2.1.is_none()
                && w2.2.is_none()
                && w2.3.is_none())
        })
        .await
}

async fn wait_for_task_completed(
    harness: &FullStackHarness,
    run_id: i32,
    timeout: Duration,
) -> anyhow::Result<()> {
    harness
        .wait_for("task completed", timeout, || async {
            let state: Option<String> = sqlx::query_scalar(
                "SELECT state FROM run_tasks WHERE run_id = $1 AND sequence_nr = 1",
            )
            .bind(run_id)
            .fetch_optional(&harness.pool)
            .await?;
            Ok(state.as_deref() == Some("completed"))
        })
        .await
}

async fn wait_for_batch_retry_count(
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

async fn wait_for_failed_batch(
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

struct SamplerCheckpointProgram<'a> {
    harness: &'a mut FullStackHarness,
    run_id: i32,
    run_name: &'a str,
    paused_current_accumulator: Option<JsonValue>,
    paused_checkpoint: Option<JsonValue>,
    paused_progress: Option<(i64, i64)>,
    paused_stage_snapshot_count: Option<i64>,
    paused_observable_snapshot_count: Option<i64>,
}

impl<'a> SamplerCheckpointProgram<'a> {
    fn new(harness: &'a mut FullStackHarness, run_id: i32, run_name: &'a str) -> Self {
        Self {
            harness,
            run_id,
            run_name,
            paused_current_accumulator: None,
            paused_checkpoint: None,
            paused_progress: None,
            paused_stage_snapshot_count: None,
            paused_observable_snapshot_count: None,
        }
    }

    async fn assign_sampler(&mut self, node_name: &str) -> anyhow::Result<()> {
        self.harness
            .cli()
            .args([
                "node",
                "assign",
                node_name,
                "sampler-aggregator",
                self.run_name,
            ])
            .assert()
            .success();
        Ok(())
    }

    async fn assign_evaluator(&mut self, node_name: &str) -> anyhow::Result<()> {
        self.harness
            .cli()
            .args(["node", "assign", node_name, "evaluator", self.run_name])
            .assert()
            .success();
        Ok(())
    }

    async fn pause_run(&mut self) -> anyhow::Result<()> {
        self.harness
            .cli()
            .args(["run", "pause", self.run_name])
            .assert()
            .success();
        Ok(())
    }

    async fn wait_sampler_active(
        &mut self,
        node_name: &str,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        let run_id = self.run_id;
        self.harness
            .wait_for(
                format!("sampler node {node_name} becomes active"),
                timeout,
                || async {
                    let state = self.harness.node_state(node_name).await?;
                    Ok(state.0 == Some(run_id)
                        && state.1.as_deref() == Some("sampler_aggregator")
                        && state.2 == Some(run_id)
                        && state.3.as_deref() == Some("sampler_aggregator"))
                },
            )
            .await
    }

    async fn wait_nodes_down(
        &mut self,
        node_names: &[&str],
        timeout: Duration,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            let mut states = Vec::new();
            let mut all_down = true;
            for node_name in node_names {
                let state = self.harness.node_state(node_name).await?;
                if state.0.is_some() || state.1.is_some() || state.2.is_some() || state.3.is_some()
                {
                    all_down = false;
                }
                states.push(((*node_name).to_string(), state));
            }
            if all_down {
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out waiting for all scripted nodes reconcile down: {states:?}"
                );
            }
            sleep(Duration::from_millis(100)).await;
        }
    }

    fn checkpoint_resume_state_matches(
        current: &Option<JsonValue>,
        paused: &Option<JsonValue>,
    ) -> bool {
        let (Some(current), Some(paused)) = (current, paused) else {
            return false;
        };
        current.get("task_id") == paused.get("task_id")
            && current.get("sampler_snapshot") == paused.get("sampler_snapshot")
            && current.get("observable_state") == paused.get("observable_state")
            && current.get("queue") == paused.get("queue")
    }

    async fn capture_paused_state(&mut self, timeout: Duration) -> anyhow::Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            let current_accumulator = self.harness.run_current_accumulator(self.run_id).await?;
            let checkpoint = self.harness.run_sampler_checkpoint(self.run_id).await?;
            let progress = self.harness.run_sample_progress(self.run_id).await?;
            let stage_snapshot_count = self.harness.run_stage_snapshot_count(self.run_id).await?;
            let observable_snapshot_count = self
                .harness
                .persisted_observable_snapshot_count(self.run_id)
                .await?;
            if current_accumulator.is_some() && checkpoint.is_some() {
                self.paused_current_accumulator = current_accumulator;
                self.paused_checkpoint = checkpoint;
                self.paused_progress = Some(progress);
                self.paused_stage_snapshot_count = Some(stage_snapshot_count);
                self.paused_observable_snapshot_count = Some(observable_snapshot_count);
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for paused sampler state is persisted");
            }
            sleep(Duration::from_millis(100)).await;
        }
    }

    async fn wait_exact_restored_state(
        &mut self,
        sampler_node_name: &str,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        let paused_current_accumulator = self.paused_current_accumulator.clone();
        let paused_checkpoint = self.paused_checkpoint.clone();
        let paused_completed_samples = self.paused_progress.map(|(_, completed)| completed);
        let paused_stage_snapshot_count = self.paused_stage_snapshot_count;
        let paused_observable_snapshot_count = self.paused_observable_snapshot_count;
        let run_id = self.run_id;
        self.harness
            .wait_for(
                "sampler checkpoint restores paused state without losing persisted snapshots",
                timeout,
                || async {
                    let sampler_state = self.harness.node_state(sampler_node_name).await?;
                    let current_accumulator = self.harness.run_current_accumulator(run_id).await?;
                    let checkpoint = self.harness.run_sampler_checkpoint(run_id).await?;
                    let progress = self.harness.run_sample_progress(run_id).await?;
                    let stage_snapshot_count =
                        self.harness.run_stage_snapshot_count(run_id).await?;
                    let observable_snapshot_count = self
                        .harness
                        .persisted_observable_snapshot_count(run_id)
                        .await?;
                    let sampler_restored = sampler_state.0 == Some(run_id)
                        && sampler_state.1.as_deref() == Some("sampler_aggregator")
                        && sampler_state.2 == Some(run_id)
                        && sampler_state.3.as_deref() == Some("sampler_aggregator");
                    let completed_restored = paused_completed_samples
                        .is_none_or(|paused_completed| progress.1 >= paused_completed);
                    let stage_snapshots_retained = paused_stage_snapshot_count
                        .is_none_or(|paused_count| stage_snapshot_count >= paused_count);
                    let observable_snapshots_retained = paused_observable_snapshot_count
                        .is_none_or(|paused_count| observable_snapshot_count >= paused_count);
                    Ok(sampler_restored
                        && current_accumulator == paused_current_accumulator
                        && Self::checkpoint_resume_state_matches(&checkpoint, &paused_checkpoint)
                        && completed_restored
                        && stage_snapshots_retained
                        && observable_snapshots_retained)
                },
            )
            .await
    }

    async fn wait_progress_advances(&mut self, timeout: Duration) -> anyhow::Result<()> {
        let Some((paused_produced, paused_completed)) = self.paused_progress else {
            anyhow::bail!("paused progress not captured before resume");
        };
        let run_id = self.run_id;
        self.harness
            .wait_for("resumed run makes forward progress", timeout, || async {
                let (nr_produced_samples, nr_completed_samples) =
                    self.harness.run_sample_progress(run_id).await?;
                Ok(
                    nr_produced_samples > paused_produced
                        || nr_completed_samples > paused_completed,
                )
            })
            .await
    }

    async fn wait_persisted_state_retained(&mut self, timeout: Duration) -> anyhow::Result<()> {
        let paused_completed_samples = self.paused_progress.map(|(_, completed)| completed);
        let paused_stage_snapshot_count = self.paused_stage_snapshot_count;
        let paused_observable_snapshot_count = self.paused_observable_snapshot_count;
        let run_id = self.run_id;
        self.harness
            .wait_for(
                "resumed run retains persisted sampler state",
                timeout,
                || async {
                    let current_accumulator = self.harness.run_current_accumulator(run_id).await?;
                    let checkpoint = self.harness.run_sampler_checkpoint(run_id).await?;
                    let progress = self.harness.run_sample_progress(run_id).await?;
                    let stage_snapshot_count =
                        self.harness.run_stage_snapshot_count(run_id).await?;
                    let observable_snapshot_count = self
                        .harness
                        .persisted_observable_snapshot_count(run_id)
                        .await?;
                    let completed_restored = paused_completed_samples
                        .is_none_or(|paused_completed| progress.1 >= paused_completed);
                    let stage_snapshots_retained = paused_stage_snapshot_count
                        .is_none_or(|paused_count| stage_snapshot_count >= paused_count);
                    let observable_snapshots_retained = paused_observable_snapshot_count
                        .is_none_or(|paused_count| observable_snapshot_count >= paused_count);
                    Ok(current_accumulator.is_some()
                        && checkpoint.is_some()
                        && completed_restored
                        && stage_snapshots_retained
                        && observable_snapshots_retained)
                },
            )
            .await
    }

    async fn resume_and_verify_exact_restore(
        &mut self,
        sampler_node_name: &str,
        evaluator_node_names: &[&str],
        timeout: Duration,
    ) -> anyhow::Result<()> {
        self.assign_sampler(sampler_node_name).await?;
        self.wait_exact_restored_state(sampler_node_name, timeout)
            .await?;
        for node_name in evaluator_node_names {
            self.assign_evaluator(node_name).await?;
        }
        self.wait_progress_advances(timeout).await?;
        Ok(())
    }

    async fn restart_and_resume_and_verify_retained_state(
        &mut self,
        sampler_node_name: &str,
        evaluator_node_names: &[&str],
        timeout: Duration,
    ) -> anyhow::Result<()> {
        self.assign_sampler(sampler_node_name).await?;
        for node_name in evaluator_node_names {
            self.assign_evaluator(node_name).await?;
        }
        self.wait_progress_advances(timeout).await?;
        self.wait_persisted_state_retained(timeout).await?;
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_flow_exercises_run_and_node_lifecycle() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let invalid_config = temp_run_add_config(
        r#"
name = "invalid-run"

[point_spec]
continuous_dims = 1
discrete_dims = 0
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(invalid_config.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "top-level [point_spec] or [domain] is no longer supported",
        ));

    let valid_config = temp_run_add_config(
        r#"
name = "full-stack-e2e"
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(valid_config.path())
        .assert()
        .success();

    let run_id: i32 = sqlx::query_scalar("SELECT id FROM runs WHERE name = 'full-stack-e2e'")
        .fetch_one(&harness.pool)
        .await?;

    harness.start_nodes(&["w-1", "w-2"]).await?;

    let node_list = harness
        .cli()
        .arg("node")
        .arg("list")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let node_list = String::from_utf8(node_list)?;
    assert!(node_list.contains("w-1"));
    assert!(node_list.contains("w-2"));
    assert!(node_list.contains("N/A"));

    harness
        .cli()
        .args(["node", "assign", "w-1", "evaluator", "full-stack-e2e"])
        .assert()
        .success();
    harness
        .cli()
        .args(["node", "assign", "w-2", "evaluator", "full-stack-e2e"])
        .assert()
        .success();

    harness
        .wait_for(
            "idle evaluator assignments are cleared",
            Duration::from_secs(10),
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

    harness
        .cli()
        .args([
            "node",
            "assign",
            "ghost-node",
            "evaluator",
            "full-stack-e2e",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("node 'ghost-node' is not live"));

    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-2",
            "sampler-aggregator",
            "full-stack-e2e",
        ])
        .assert()
        .success();

    harness
        .wait_for(
            "idle sampler assignment clears idle run assignments",
            Duration::from_secs(10),
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

    let missing_run_id = run_id + 10_000;
    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-1",
            "evaluator",
            &missing_run_id.to_string(),
        ])
        .assert()
        .failure();

    harness
        .cli()
        .args(["node", "assign", "w-1", "evaluator", "full-stack-e2e"])
        .assert()
        .success();
    harness
        .cli()
        .args(["node", "assign", "w-2", "evaluator", "full-stack-e2e"])
        .assert()
        .success();

    harness
        .wait_for(
            "idle control loop clears reassigned evaluators",
            Duration::from_secs(10),
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

    harness
        .cli()
        .args(["run", "pause", "full-stack-e2e"])
        .assert()
        .success();

    harness
        .wait_for(
            "paused run reconciles all nodes down",
            Duration::from_secs(10),
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

    harness
        .cli()
        .args(["node", "assign", "w-1", "evaluator", "full-stack-e2e"])
        .assert()
        .success();
    harness
        .cli()
        .args(["node", "assign", "w-2", "evaluator", "full-stack-e2e"])
        .assert()
        .success();

    harness
        .wait_for(
            "paused idle run clears reassigned evaluators",
            Duration::from_secs(10),
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

    harness
        .cli()
        .args(["run", "pause", "full-stack-e2e"])
        .assert()
        .success();
    harness
        .wait_for(
            "second pause clears current state",
            Duration::from_secs(10),
            || async {
                let w1 = harness.node_state("w-1").await?;
                let w2 = harness.node_state("w-2").await?;
                Ok(w1.2.is_none() && w1.3.is_none() && w2.2.is_none() && w2.3.is_none())
            },
        )
        .await?;

    harness
        .cli()
        .args(["run", "remove", "full-stack-e2e"])
        .assert()
        .success();

    let remaining_runs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runs")
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(remaining_runs, 0);

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_rejects_first_sample_without_accumulator_state() -> anyhow::Result<()> {
    let harness = FullStackHarness::new().await?;
    let config = temp_run_add_config(
        r#"
name = "missing-accumulator-e2e"

[[task_queue]]
name = "sample-a"
kind = "sample"
stop_condition = { max_samples = 16 }
sampler_aggregator = { config = { kind = "naive_monte_carlo", seed = 0 } }
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "sample task has no effective accumulator configuration",
        ));

    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_set_accumulator_enables_following_sample() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    let config = temp_run_add_config(
        r#"
name = "set-accumulator-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[evaluator_runner_params]
min_tick_time_ms = 50
db_pool_size = 1

[sampler_aggregator_runner_params]
min_tick_time_ms = 10
db_pool_size = 1

[[task_queue]]
name = "prep"
kind = "set_accumulator"
accumulator = "scalar"

[[task_queue]]
name = "sample-a"
kind = "sample"
stop_condition = { max_samples = 32 }
sampler_aggregator = { config = { kind = "naive_monte_carlo", seed = 0 } }
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 = sqlx::query_scalar("SELECT id FROM runs WHERE name = 'set-accumulator-e2e'")
        .fetch_one(&harness.pool)
        .await?;

    harness.start_nodes(&["w-1", "w-2"]).await?;
    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-1",
            "sampler-aggregator",
            "set-accumulator-e2e",
        ])
        .assert()
        .success();
    harness
        .cli()
        .args(["node", "assign", "w-2", "evaluator", "set-accumulator-e2e"])
        .assert()
        .success();

    harness
        .wait_for(
            "set_accumulator task completes",
            Duration::from_secs(10),
            || async {
                let state: String = sqlx::query_scalar(
                    "SELECT state FROM run_tasks WHERE run_id = $1 AND name = 'prep'",
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
            "sample task completes after set_accumulator",
            Duration::from_secs(20),
            || async {
                let state: String = sqlx::query_scalar(
                    "SELECT state FROM run_tasks WHERE run_id = $1 AND name = 'sample-a'",
                )
                .bind(run_id)
                .fetch_one(&harness.pool)
                .await?;
                Ok(state == "completed")
            },
        )
        .await?;

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege and nginx"]
async fn full_stack_deploy_can_run_two_port_isolated_instances() -> anyhow::Result<()> {
    if !nginx_available() {
        eprintln!("skipping deploy isolation e2e because nginx is not available");
        return Ok(());
    }

    let mut harness = FullStackHarness::new().await?;
    let second_db = TestDatabase::create().await?;
    let frontend_build = temp_frontend_build();
    let password_hash = hash_password_for_tests("test-password");
    let server_config = temp_server_config(
        "127.0.0.1",
        4000,
        "http://localhost:8080",
        false,
        true,
        (&password_hash, "test-session-secret"),
    );
    let frontend_port_a = unused_local_port()?;
    let frontend_port_b = unused_local_port()?;
    let deploy_config_a =
        temp_deploy_server_config(frontend_build.path(), server_config.path(), frontend_port_a);
    let deploy_config_b =
        temp_deploy_server_config(frontend_build.path(), server_config.path(), frontend_port_b);
    let api_port_a = unused_local_port()?;
    let api_port_b = unused_local_port()?;

    for (label, database_url, frontend_port, api_port, deploy_config_path) in [
        (
            "deploy-a",
            harness.db.database_url.as_str(),
            frontend_port_a,
            api_port_a,
            deploy_config_a.path(),
        ),
        (
            "deploy-b",
            second_db.database_url.as_str(),
            frontend_port_b,
            api_port_b,
            deploy_config_b.path(),
        ),
    ] {
        let mut child = TokioCommand::new(&harness.bin_path);
        child
            .arg("--runtime-config")
            .arg(&harness.runtime_config_path)
            .arg("--database-url")
            .arg(database_url)
            .arg("deploy")
            .arg("run")
            .arg("--server-config")
            .arg(deploy_config_path)
            .arg("--api-port")
            .arg(api_port.to_string())
            .arg("--allowed-origin")
            .arg(format!("http://localhost:{frontend_port}"))
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        let child = child.spawn()?;
        harness.children.push(ManagedChild {
            label: label.to_string(),
            child,
        });
    }

    let base_a = format!("http://127.0.0.1:{frontend_port_a}");
    let base_b = format!("http://127.0.0.1:{frontend_port_b}");
    harness
        .wait_for("first deploy API health", Duration::from_secs(20), || {
            let base = base_a.clone();
            async move {
                Ok(http_get(&base, "/api/health")
                    .await
                    .is_ok_and(|response| response.contains("\"status\":\"ok\"")))
            }
        })
        .await?;
    harness
        .wait_for("second deploy API health", Duration::from_secs(20), || {
            let base = base_b.clone();
            async move {
                Ok(http_get(&base, "/api/health")
                    .await
                    .is_ok_and(|response| response.contains("\"status\":\"ok\"")))
            }
        })
        .await?;

    assert!(http_get(&base_a, "/").await?.contains("gammaboard e2e"));
    assert!(http_get(&base_b, "/").await?.contains("gammaboard e2e"));

    harness.terminate_child("deploy-a").await?;
    harness.terminate_child("deploy-b").await?;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    second_db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_pause_resume_restores_sampler_checkpoint() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config = temp_run_add_config(
        r#"
name = "sampler-checkpoint-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
name = "train-a"
kind = "sample"
stop_condition = { max_samples = 100000000 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 =
        sqlx::query_scalar("SELECT id FROM runs WHERE name = 'sampler-checkpoint-e2e'")
            .fetch_one(&harness.pool)
            .await?;

    {
        let mut program =
            SamplerCheckpointProgram::new(&mut harness, run_id, "sampler-checkpoint-e2e");

        program.harness.start_nodes(&["w-1", "w-2", "w-3"]).await?;

        program.assign_sampler("w-1").await?;
        program.assign_evaluator("w-2").await?;
        program.assign_evaluator("w-3").await?;

        program
            .wait_sampler_active("w-1", Duration::from_secs(15))
            .await?;

        tokio::time::sleep(Duration::from_secs(2)).await;

        program.pause_run().await?;
        program
            .wait_nodes_down(&["w-1", "w-2", "w-3"], Duration::from_secs(15))
            .await?;
        program
            .capture_paused_state(Duration::from_secs(15))
            .await?;

        program
            .resume_and_verify_exact_restore("w-1", &["w-2", "w-3"], Duration::from_secs(15))
            .await?;
    }

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_server_can_restart_while_nodes_keep_running() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let server_url = harness.start_server().await?;
    harness.start_nodes(&["w-1", "w-2"]).await?;

    harness
        .wait_for(
            "nodes visible through server api",
            Duration::from_secs(10),
            || {
                let server_url = server_url.clone();
                async move {
                    let body = http_get(&server_url, "/api/nodes").await?;
                    Ok(body.contains("\"node_name\":\"w-1\"")
                        && body.contains("\"node_name\":\"w-2\""))
                }
            },
        )
        .await?;

    let server_label = server_url.trim_start_matches("http://").to_string();
    harness
        .kill_child(&format!("server:{server_label}"))
        .await?;

    let restarted_server_url = harness.start_server().await?;
    harness
        .wait_for(
            "nodes visible after server restart",
            Duration::from_secs(10),
            || {
                let server_url = restarted_server_url.clone();
                async move {
                    let health = http_get(&server_url, "/api/health").await?;
                    let nodes = http_get(&server_url, "/api/nodes").await?;
                    Ok(health.contains("\"status\":\"ok\"")
                        && nodes.contains("\"node_name\":\"w-1\"")
                        && nodes.contains("\"node_name\":\"w-2\""))
                }
            },
        )
        .await?;

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_run_node_exits_on_sigterm_and_releases_name() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    let config = temp_run_add_config(
        r#"
name = "sigterm-node-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 1_000_000 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"#,
    );
    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 = sqlx::query_scalar("SELECT id FROM runs WHERE name = 'sigterm-node-e2e'")
        .fetch_one(&harness.pool)
        .await?;

    harness.start_node("w-1").await?;
    harness
        .cli()
        .args(["node", "assign", "w-1", "evaluator", "sigterm-node-e2e"])
        .assert()
        .success();

    harness
        .wait_for(
            "node has active evaluator assignment before sigterm",
            Duration::from_secs(10),
            || async {
                let state = harness.node_state("w-1").await?;
                Ok(state.2 == Some(run_id) && state.3.as_deref() == Some("evaluator"))
            },
        )
        .await?;

    harness.terminate_child("w-1").await?;

    harness
        .wait_for(
            "node lease expired after sigterm",
            Duration::from_secs(10),
            || {
                let pool = harness.pool.clone();
                async move {
                    let count: i64 = sqlx::query_scalar(
                        "SELECT COUNT(*) FROM nodes WHERE name = $1 AND lease_expires_at > now()",
                    )
                    .bind("w-1")
                    .fetch_one(&pool)
                    .await?;
                    Ok(count == 0)
                }
            },
        )
        .await?;

    let state = harness.node_state("w-1").await?;
    assert_eq!(state.0, None);
    assert_eq!(state.1, None);
    assert_eq!(state.2, None);
    assert_eq!(state.3, None);

    harness.start_node("w-1").await?;

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_server_queue_tuning_update_applies_to_active_sample_task() -> anyhow::Result<()>
{
    let mut harness = FullStackHarness::new().await?;

    let config = temp_run_add_config(
        r#"
name = "queue-tuning-live-update-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0
min_eval_time_per_sample_ms = 5

[[task_queue]]
name = "sample-a"
kind = "sample"
stop_condition = { max_samples = 8192 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }

[sampler_aggregator_runner_params]
performance_snapshot_interval_ms = 100
min_tick_time_ms = 10
frontend_sync_interval_ms = 100

[sampler_aggregator_runner_params.queue]
queue_buffer = 1.0
target_batch_eval_ms = 50.0
max_batch_size = 32
local_pending_buffer_multiplier = 1.0
max_queue_size = 64
max_batches_per_tick = 8
max_insert_bundle_size = 8
max_concurrent_insert_tasks = 2
completed_batch_fetch_limit = 64
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 = sqlx::query_scalar("SELECT id FROM runs WHERE name = $1")
        .bind("queue-tuning-live-update-e2e")
        .fetch_one(&harness.pool)
        .await?;
    let task_id: i64 = sqlx::query_scalar(
        "SELECT id FROM run_tasks WHERE run_id = $1 AND name = 'sample-a' LIMIT 1",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;

    let password = "operator-secret";
    let password_hash = hash_password_for_tests(password);
    let server_url = harness
        .start_server_with_auth((&password_hash, "test-session-secret"))
        .await?;

    let login = http_post_json(
        &server_url,
        "/api/auth/login",
        json!({ "password": password }),
        None,
    )
    .await?;
    assert_eq!(login.status(), reqwest::StatusCode::OK);
    let cookie = login
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or("").to_string())
        .ok_or_else(|| anyhow::anyhow!("missing session cookie"))?;

    harness.start_nodes(&["w-1", "w-2"]).await?;

    let assign_sampler = http_post_json(
        &server_url,
        "/api/nodes/w-1/assign",
        json!({ "run_id": run_id, "role": "sampler_aggregator" }),
        Some(&cookie),
    )
    .await?;
    assert_eq!(assign_sampler.status(), reqwest::StatusCode::OK);

    let assign_eval = http_post_json(
        &server_url,
        "/api/nodes/w-2/assign",
        json!({ "run_id": run_id, "role": "evaluator" }),
        Some(&cookie),
    )
    .await?;
    assert_eq!(assign_eval.status(), reqwest::StatusCode::OK);

    harness
        .wait_for(
            "sample task becomes active",
            Duration::from_secs(20),
            || {
                let pool = harness.pool.clone();
                async move {
                    let state: Option<String> = sqlx::query_scalar(
                        "SELECT state FROM run_tasks WHERE run_id = $1 AND name = 'sample-a'",
                    )
                    .bind(run_id)
                    .fetch_optional(&pool)
                    .await?;
                    Ok(state.as_deref() == Some("active"))
                }
            },
        )
        .await?;

    harness
        .wait_for(
            "initial sampler diagnostics persisted",
            Duration::from_secs(20),
            || {
                let pool = harness.pool.clone();
                async move {
                    let diag: Option<JsonValue> = sqlx::query_scalar(
                        r#"
                    SELECT engine_diagnostics
                    FROM sampler_aggregator_performance_latest
                    WHERE run_id = $1 AND worker_id = 'w-1'
                    "#,
                    )
                    .bind(run_id)
                    .fetch_optional(&pool)
                    .await?;
                    let Some(diag) = diag else {
                        return Ok(false);
                    };
                    Ok(diag["runner"]["queue_buffer"].as_f64() == Some(1.0))
                }
            },
        )
        .await?;

    let update = http_post_json(
        &server_url,
        &format!("/api/runs/{run_id}/tasks/{task_id}/queue-tuning"),
        json!({
            "queue_tuning": {
                "queue_buffer": 0.0,
                "max_batches_per_tick": 1,
                "completed_batch_fetch_limit": 7
            }
        }),
        Some(&cookie),
    )
    .await?;
    assert_eq!(update.status(), reqwest::StatusCode::OK);

    harness
        .wait_for(
            "updated queue tuning reflected in active task payload",
            Duration::from_secs(20),
            || {
                let pool = harness.pool.clone();
                async move {
                    let task: JsonValue = sqlx::query_scalar(
                        "SELECT task FROM run_tasks WHERE run_id = $1 AND id = $2",
                    )
                    .bind(run_id)
                    .bind(task_id)
                    .fetch_one(&pool)
                    .await?;
                    Ok(task["queue_tuning"]["queue_buffer"].as_f64() == Some(0.0)
                        && task["queue_tuning"]["max_batches_per_tick"].as_u64() == Some(1)
                        && task["queue_tuning"]["completed_batch_fetch_limit"].as_u64() == Some(7))
                }
            },
        )
        .await?;

    harness
        .wait_for(
            "updated queue tuning reflected in live runner diagnostics",
            Duration::from_secs(20),
            || {
                let pool = harness.pool.clone();
                async move {
                    let diag: Option<JsonValue> = sqlx::query_scalar(
                        r#"
                        SELECT engine_diagnostics
                        FROM sampler_aggregator_performance_latest
                        WHERE run_id = $1 AND worker_id = 'w-1'
                        "#,
                    )
                    .bind(run_id)
                    .fetch_optional(&pool)
                    .await?;
                    let Some(diag) = diag else {
                        return Ok(false);
                    };
                    Ok(diag["runner"]["queue_buffer"].as_f64() == Some(0.0)
                        && diag["runner"]["target_pending_batches"].as_u64() == Some(0))
                }
            },
        )
        .await?;

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_removes_assigned_run_immediately() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config = temp_run_add_config(
        r#"
name = "delete-assigned-run-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0
min_eval_time_per_sample_ms = 20

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = 100_000 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 =
        sqlx::query_scalar("SELECT id FROM runs WHERE name = 'delete-assigned-run-e2e'")
            .fetch_one(&harness.pool)
            .await?;

    harness.start_nodes(&["w-1", "w-2"]).await?;
    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-1",
            "sampler-aggregator",
            "delete-assigned-run-e2e",
        ])
        .assert()
        .success();
    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-2",
            "evaluator",
            "delete-assigned-run-e2e",
        ])
        .assert()
        .success();

    harness
        .wait_for(
            "workers become active before delete",
            Duration::from_secs(15),
            || async {
                let active_count: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM nodes WHERE active_run_id = $1 OR desired_run_id = $1",
                )
                .bind(run_id)
                .fetch_one(&harness.pool)
                .await?;
                Ok(active_count >= 2)
            },
        )
        .await?;

    harness
        .cli()
        .args(["run", "remove", "delete-assigned-run-e2e"])
        .assert()
        .success();

    harness
        .wait_for("run removed and workers unassigned", Duration::from_secs(15), || async {
            let run_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runs WHERE id = $1")
                .bind(run_id)
                .fetch_one(&harness.pool)
                .await?;
            let assigned_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM nodes WHERE desired_run_id IS NOT NULL OR active_run_id IS NOT NULL",
            )
            .fetch_one(&harness.pool)
            .await?;
            Ok(run_count == 0 && assigned_count == 0)
        })
        .await?;

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_removes_child_runs_with_parent() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness
        .start_nodes(&["delete-family-s", "delete-family-e"])
        .await?;

    let config = temp_run_add_config(
        r#"
name = "delete-parent-run-e2e"

[evaluator_runner_params]
min_tick_time_ms = 50
db_pool_size = 1

[sampler_aggregator_runner_params]
min_tick_time_ms = 10
db_pool_size = 1

[[task_queue]]
name = "scan"
kind = "parameter_scan"
max_concurrent_runs = 1
trial_run_toml = """
name = "delete-child-run-$(scale:1)"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0
min_eval_time_per_sample_ms = 20

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 1_000_000 }
measurement = { quantity = "central_value" }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"""

[task_queue.parameter]
name = "scale"
values = [1]

[task_queue.measurement]
source_task = "sample"
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let parent_run_id: i32 =
        sqlx::query_scalar("SELECT id FROM runs WHERE name = 'delete-parent-run-e2e'")
            .fetch_one(&harness.pool)
            .await?;

    harness
        .cli()
        .args(["auto-assign", &parent_run_id.to_string()])
        .assert()
        .success();

    harness
        .wait_for(
            "scan child run exists before parent delete",
            Duration::from_secs(30),
            || async {
                let child_count: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM runs WHERE parent_run_id = $1")
                        .bind(parent_run_id)
                        .fetch_one(&harness.pool)
                        .await?;
                Ok(child_count == 1)
            },
        )
        .await?;

    let child_run_id: i32 = sqlx::query_scalar(
        r#"
        SELECT id
        FROM runs
        WHERE parent_run_id = $1
          AND spawn_kind = 'parameter_scan'
        "#,
    )
    .bind(parent_run_id)
    .fetch_one(&harness.pool)
    .await?;

    harness
        .cli()
        .args(["run", "remove", &parent_run_id.to_string()])
        .assert()
        .success();

    harness
        .wait_for(
            "parent delete removes child runs and assignments",
            Duration::from_secs(15),
            || async {
                let remaining_runs: i64 = sqlx::query_scalar(
                    r#"
                    SELECT COUNT(*)
                    FROM runs
                    WHERE id = $1 OR id = $2 OR parent_run_id = $1
                    "#,
                )
                .bind(parent_run_id)
                .bind(child_run_id)
                .fetch_one(&harness.pool)
                .await?;
                let remaining_assignments: i64 = sqlx::query_scalar(
                    r#"
                    SELECT COUNT(*)
                    FROM nodes
                    WHERE desired_run_id IN ($1, $2)
                       OR active_run_id IN ($1, $2)
                    "#,
                )
                .bind(parent_run_id)
                .bind(child_run_id)
                .fetch_one(&harness.pool)
                .await?;
                Ok(remaining_runs == 0 && remaining_assignments == 0)
            },
        )
        .await?;

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_graceful_node_shutdown_waits_for_sampler_unassign() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config = temp_run_add_config(
        r#"
name = "graceful-node-shutdown-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = 10_000_000 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 =
        sqlx::query_scalar("SELECT id FROM runs WHERE name = 'graceful-node-shutdown-e2e'")
            .fetch_one(&harness.pool)
            .await?;

    {
        let mut program =
            SamplerCheckpointProgram::new(&mut harness, run_id, "graceful-node-shutdown-e2e");

        program.harness.start_nodes(&["w-1", "w-2"]).await?;

        program.assign_sampler("w-1").await?;
        program.assign_evaluator("w-2").await?;
        program
            .wait_sampler_active("w-1", Duration::from_secs(15))
            .await?;

        tokio::time::sleep(Duration::from_secs(2)).await;

        let store = gammaboard::init_pg_store(&program.harness.db.database_url, 10).await?;
        let result = node_api::stop_all_nodes_gracefully(
            &store,
            node_api::GracefulNodeShutdownParams {
                sampler_drain_timeout: Duration::from_secs(10),
                node_stop_timeout: Duration::from_secs(10),
                poll_interval: Duration::from_millis(50),
            },
        )
        .await?;

        assert!(!result.sampler_drain_timed_out);
        assert!(result.assignments_cleared >= 2);
        assert_eq!(result.active_samplers_remaining, 0);

        program
            .wait_nodes_down(&["w-1", "w-2"], Duration::from_secs(15))
            .await?;
        program.harness.reap_children(&["w-1", "w-2"]).await?;
        program
            .capture_paused_state(Duration::from_secs(15))
            .await?;
        program.harness.start_nodes(&["w-1", "w-2"]).await?;
        program
            .restart_and_resume_and_verify_retained_state("w-1", &["w-2"], Duration::from_secs(15))
            .await?;
    }

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_server_auth_protects_pause_endpoint() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config = temp_run_add_config("name = \"auth-e2e\"\n");
    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 = sqlx::query_scalar("SELECT id FROM runs WHERE name = 'auth-e2e'")
        .fetch_one(&harness.pool)
        .await?;

    harness.start_node("w-1").await?;
    harness
        .cli()
        .args(["node", "assign", "w-1", "evaluator", "auth-e2e"])
        .assert()
        .success();

    harness
        .wait_for(
            "node assigned for auth test",
            Duration::from_secs(10),
            || async {
                let state = harness.node_state("w-1").await?;
                Ok(state.0 == Some(run_id) && state.1.as_deref() == Some("evaluator"))
            },
        )
        .await?;

    let password = "operator-secret";
    let password_hash = hash_password_for_tests(password);
    let server_url = harness
        .start_server_with_auth((&password_hash, "test-session-secret"))
        .await?;

    let runs = http_get(&server_url, "/api/runs").await?;
    assert!(runs.contains("\"run_name\":\"auth-e2e\""));

    let unauthorized = http_post_json(
        &server_url,
        &format!("/api/runs/{run_id}/pause"),
        json!({}),
        None,
    )
    .await?;
    assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);

    let login = http_post_json(
        &server_url,
        "/api/auth/login",
        json!({ "password": password }),
        None,
    )
    .await?;
    assert_eq!(login.status(), reqwest::StatusCode::OK);
    let cookie = login
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or("").to_string())
        .ok_or_else(|| anyhow::anyhow!("missing session cookie"))?;

    let pause = http_post_json(
        &server_url,
        &format!("/api/runs/{run_id}/pause"),
        json!({}),
        Some(&cookie),
    )
    .await?;
    assert_eq!(pause.status(), reqwest::StatusCode::OK);

    harness
        .wait_for(
            "authenticated pause clears desired assignment",
            Duration::from_secs(10),
            || async {
                let state = harness.node_state("w-1").await?;
                Ok(state.0.is_none() && state.1.is_none())
            },
        )
        .await?;

    let assign = http_post_json(
        &server_url,
        "/api/nodes/w-1/assign",
        json!({ "run_id": run_id, "role": "evaluator" }),
        Some(&cookie),
    )
    .await?;
    assert_eq!(assign.status(), reqwest::StatusCode::OK);

    harness
        .wait_for(
            "authenticated assign restores desired assignment",
            Duration::from_secs(10),
            || async {
                let state = harness.node_state("w-1").await?;
                Ok(state.0 == Some(run_id) && state.1.as_deref() == Some("evaluator"))
            },
        )
        .await?;

    harness.start_node("w-2").await?;
    let auto_assign = http_post_json(
        &server_url,
        &format!("/api/runs/{run_id}/auto-assign"),
        json!({ "max_evaluators": 1 }),
        Some(&cookie),
    )
    .await?;
    assert_eq!(auto_assign.status(), reqwest::StatusCode::OK);

    harness
        .wait_for(
            "authenticated auto-assign sets desired assignments",
            Duration::from_secs(10),
            || async {
                let w1 = harness.node_state("w-1").await?;
                let w2 = harness.node_state("w-2").await?;
                Ok(
                    (w1.0 == Some(run_id) && w1.1.as_deref() == Some("sampler_aggregator"))
                        || (w2.0 == Some(run_id) && w2.1.as_deref() == Some("sampler_aggregator")),
                )
            },
        )
        .await?;

    let unassign = http_post_json(
        &server_url,
        "/api/nodes/w-1/unassign",
        json!({}),
        Some(&cookie),
    )
    .await?;
    assert_eq!(unassign.status(), reqwest::StatusCode::OK);

    harness
        .wait_for(
            "authenticated unassign clears desired assignment",
            Duration::from_secs(10),
            || async {
                let state = harness.node_state("w-1").await?;
                Ok(state.0.is_none() && state.1.is_none())
            },
        )
        .await?;

    let stop = http_post_json(&server_url, "/api/nodes/w-1/stop", json!({}), Some(&cookie)).await?;
    assert_eq!(stop.status(), reqwest::StatusCode::OK);
    let stop_body: JsonValue = serde_json::from_str(&stop.text().await?)?;
    assert_eq!(stop_body["node_name"].as_str(), Some("w-1"));
    assert_eq!(stop_body["rows_updated"].as_u64(), Some(1));

    harness
        .wait_for(
            "authenticated stop expires node lease",
            Duration::from_secs(10),
            || async {
                let live: bool = sqlx::query_scalar(
                    "SELECT lease_expires_at > now() FROM nodes WHERE name = 'w-1'",
                )
                .fetch_one(&harness.pool)
                .await?;
                Ok(!live)
            },
        )
        .await?;

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_server_queues_node_launch_requests_when_local_spawn_disabled()
-> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let password = "operator-secret";
    let password_hash = hash_password_for_tests(password);
    let server_url = harness
        .start_server_with_auth_and_local_spawn((&password_hash, "test-session-secret"), false)
        .await?;

    let login = http_post_json(
        &server_url,
        "/api/auth/login",
        json!({ "password": password }),
        None,
    )
    .await?;
    assert_eq!(login.status(), reqwest::StatusCode::OK);
    let cookie = login
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or("").to_string())
        .ok_or_else(|| anyhow::anyhow!("missing session cookie"))?;

    let response = http_post_json(
        &server_url,
        "/api/nodes/auto-run",
        json!({
            "count": 2,
            "name_prefix": "queued-w",
            "args": {
                "partition": "epyc2"
            }
        }),
        Some(&cookie),
    )
    .await
    .map_err(|err| anyhow::anyhow!("node launch request failed: {err}"))?;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: JsonValue = serde_json::from_str(&response.text().await?)?;
    assert_eq!(body["started"].as_u64(), Some(0));
    assert_eq!(body["request"]["state"].as_str(), Some("pending"));
    assert_eq!(body["request"]["backend"].as_str(), Some("external"));
    assert_eq!(body["request"]["requested_count"].as_u64(), Some(2));
    assert_eq!(body["request"]["name_prefix"].as_str(), None);
    assert_eq!(
        body["request"]["args"]["groups"][0]["name_prefix"].as_str(),
        Some("queued-w")
    );
    assert_eq!(body["request"]["args"]["partition"].as_str(), Some("epyc2"));

    let node_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM nodes")
        .fetch_one(&harness.pool)
        .await
        .map_err(|err| anyhow::anyhow!("node count query failed: {err}"))?;
    assert_eq!(node_count, 0);

    let request_id = body["request"]["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing launch request id"))?
        .to_string();
    let request_id_i64 = request_id
        .parse::<i64>()
        .map_err(|err| anyhow::anyhow!("invalid launch request id: {err}"))?;
    let (state, backend): (String, String) =
        sqlx::query_as("SELECT state, backend FROM node_launch_requests WHERE id = $1")
            .bind(request_id_i64)
            .fetch_one(&harness.pool)
            .await
            .map_err(|err| anyhow::anyhow!("launch request query failed: {err}"))?;
    assert_eq!(state, "pending");
    assert_eq!(backend, "external");

    let claim_response = http_post_json(
        &server_url,
        "/api/node-launch-requests/claim-external",
        json!({}),
        Some(&cookie),
    )
    .await?;
    assert_eq!(claim_response.status(), reqwest::StatusCode::OK);
    let claim_body: JsonValue = serde_json::from_str(&claim_response.text().await?)?;
    assert_eq!(
        claim_body["request"]["id"].as_str(),
        Some(request_id.as_str())
    );
    assert_eq!(claim_body["request"]["state"].as_str(), Some("starting"));

    let workers = json!([
        { "node_name": "queued-w-1", "job_id": "test-job-1" },
        { "node_name": "queued-w-2", "job_id": "test-job-2" }
    ]);
    let progress_response = http_post_json(
        &server_url,
        &format!("/api/node-launch-requests/{request_id}/progress"),
        json!({
            "state": "starting",
            "started_count": 2,
            "result": { "workers": workers }
        }),
        Some(&cookie),
    )
    .await?;
    assert_eq!(progress_response.status(), reqwest::StatusCode::OK);

    harness.start_nodes(&["queued-w-1", "queued-w-2"]).await?;
    harness
        .wait_for(
            "launch request reconciles to running from live node leases",
            Duration::from_secs(10),
            || async {
                let body =
                    http_get_with_cookie(&server_url, "/api/node-launch-requests", &cookie).await?;
                let body: JsonValue = serde_json::from_str(&body)?;
                let state = body["items"]
                    .as_array()
                    .and_then(|items| {
                        items
                            .iter()
                            .find(|item| item["id"].as_str() == Some(request_id.as_str()))
                    })
                    .and_then(|item| item["state"].as_str());
                Ok(state == Some("running"))
            },
        )
        .await?;

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_lists_duplicate_run_names_and_reports_ambiguity() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config_a = temp_run_add_config("name = \"duplicate-run\"\n");
    let config_b = temp_run_add_config("name = \"duplicate-run\"\n");

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config_a.path())
        .assert()
        .success();
    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config_b.path())
        .assert()
        .success();

    let rows = sqlx::query("SELECT id FROM runs WHERE name = 'duplicate-run' ORDER BY id ASC")
        .fetch_all(&harness.pool)
        .await?;
    assert_eq!(rows.len(), 2);
    let id_a: i32 = rows[0].try_get("id")?;
    let id_b: i32 = rows[1].try_get("id")?;

    let list_output = harness
        .cli()
        .args(["run", "list", "duplicate-run"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let list_output = String::from_utf8(list_output)?;
    assert!(list_output.contains("duplicate-run"));
    assert!(list_output.contains(&id_a.to_string()));
    assert!(list_output.contains(&id_b.to_string()));

    harness
        .cli()
        .args(["run", "pause", "duplicate-run"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "run name 'duplicate-run' matches multiple runs",
        ))
        .stderr(predicate::str::contains(&format!("id={id_a}")))
        .stderr(predicate::str::contains(&format!("id={id_b}")));

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_reclaims_claimed_batches_after_worker_death() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config = temp_run_add_config(
        r#"
name = "worker-death-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0
min_eval_time_per_sample_ms = 100

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = 128 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }

[evaluator_runner_params]
performance_snapshot_interval_ms = 200
min_tick_time_ms = 50

[sampler_aggregator_runner_params]
performance_snapshot_interval_ms = 200
min_tick_time_ms = 10
frontend_sync_interval_ms = 1000
target_batch_eval_ms = 250.0
queue_buffer = 1.0
max_batch_size = 16
max_batches_per_tick = 4
max_queue_size = 32
completed_batch_fetch_limit = 64
strict_batch_ordering = true
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 = sqlx::query_scalar("SELECT id FROM runs WHERE name = 'worker-death-e2e'")
        .fetch_one(&harness.pool)
        .await?;

    harness.start_nodes(&["w-1", "w-2", "w-3"]).await?;

    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-1",
            "sampler-aggregator",
            "worker-death-e2e",
        ])
        .assert()
        .success();
    harness
        .cli()
        .args(["node", "assign", "w-2", "evaluator", "worker-death-e2e"])
        .assert()
        .success();

    harness
        .wait_for(
            "batch claimed by evaluator before death",
            Duration::from_secs(15),
            || {
                let pool = harness.pool.clone();
                async move {
                    let claimed: i64 = sqlx::query_scalar(
                        "SELECT COUNT(*) FROM batches WHERE run_id = $1 AND status = 'claimed' AND claimed_by_node_name = 'w-2'",
                    )
                    .bind(run_id)
                    .fetch_one(&pool)
                    .await?;
                    Ok(claimed > 0)
                }
            },
        )
        .await?;

    harness.kill_child("w-2").await?;

    harness
        .cli()
        .args(["node", "assign", "w-3", "evaluator", "worker-death-e2e"])
        .assert()
        .success();

    harness
        .wait_for(
            "dead worker lease expires and claimed batches are reclaimed",
            Duration::from_secs(45),
            || {
                let pool = harness.pool.clone();
                async move {
                    let expired: bool = sqlx::query_scalar(
                        "SELECT lease_expires_at <= now() FROM nodes WHERE name = 'w-2'",
                    )
                    .fetch_one(&pool)
                    .await?;
                    let stuck_claims: i64 = sqlx::query_scalar(
                        "SELECT COUNT(*) FROM batches WHERE run_id = $1 AND claimed_by_node_name = 'w-2'",
                    )
                    .bind(run_id)
                    .fetch_one(&pool)
                    .await?;
                    Ok(expired && stuck_claims == 0)
                }
            },
        )
        .await?;

    harness
        .wait_for(
            "replacement evaluator finishes reopened work",
            Duration::from_secs(45),
            || async {
                let w1 = harness.node_state("w-1").await?;
                let w3 = harness.node_state("w-3").await?;
                let pending_or_claimed: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM batches WHERE run_id = $1 AND status IN ('pending', 'claimed')",
                )
                .bind(run_id)
                .fetch_one(&harness.pool)
                .await?;
                Ok(w1.0.is_none()
                    && w1.1.is_none()
                    && w1.2.is_none()
                    && w1.3.is_none()
                    && w3.0.is_none()
                    && w3.1.is_none()
                    && w3.2.is_none()
                    && w3.3.is_none()
                    && pending_or_claimed == 0)
            },
        )
        .await?;

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_fails_task_gracefully_on_sampler_error() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config = temp_run_add_config(
        r#"
name = "sampler-error-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = 32 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo", fail_on_produce_batch_nr = 1 } }
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 = sqlx::query_scalar("SELECT id FROM runs WHERE name = 'sampler-error-e2e'")
        .fetch_one(&harness.pool)
        .await?;

    harness.start_nodes(&["w-1", "w-2"]).await?;

    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-1",
            "sampler-aggregator",
            "sampler-error-e2e",
        ])
        .assert()
        .success();
    harness
        .cli()
        .args(["node", "assign", "w-2", "evaluator", "sampler-error-e2e"])
        .assert()
        .success();

    wait_for_task_failed_and_run_unassigned(&harness, run_id, Duration::from_secs(30)).await?;

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_retries_batch_after_materializer_error() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config = temp_run_add_config(
        r#"
name = "materializer-error-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = 32 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo", fail_on_materialize_batch_nr = 1 } }
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 =
        sqlx::query_scalar("SELECT id FROM runs WHERE name = 'materializer-error-e2e'")
            .fetch_one(&harness.pool)
            .await?;

    harness.start_nodes(&["w-1", "w-2"]).await?;

    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-1",
            "sampler-aggregator",
            "materializer-error-e2e",
        ])
        .assert()
        .success();
    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-2",
            "evaluator",
            "materializer-error-e2e",
        ])
        .assert()
        .success();

    wait_for_batch_retry_count(&harness, run_id, 1, Duration::from_secs(30)).await?;
    wait_for_task_completed(&harness, run_id, Duration::from_secs(40)).await?;

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_retries_batch_after_evaluator_error() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config = temp_run_add_config(
        r#"
name = "evaluator-error-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0
fail_on_batch_nrs = [1]

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = 32 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 = sqlx::query_scalar("SELECT id FROM runs WHERE name = 'evaluator-error-e2e'")
        .fetch_one(&harness.pool)
        .await?;

    harness.start_nodes(&["w-1", "w-2"]).await?;

    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-1",
            "sampler-aggregator",
            "evaluator-error-e2e",
        ])
        .assert()
        .success();
    harness
        .cli()
        .args(["node", "assign", "w-2", "evaluator", "evaluator-error-e2e"])
        .assert()
        .success();

    wait_for_batch_retry_count(&harness, run_id, 1, Duration::from_secs(30)).await?;
    wait_for_task_completed(&harness, run_id, Duration::from_secs(40)).await?;

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_evaluator_batch_fails_twice_then_task_recovers() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config = temp_run_add_config(
        r#"
name = "evaluator-retry-twice-then-recover-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0
fail_on_batch_nrs = [1, 2]

[sampler_aggregator_runner_params.queue]
max_batch_retries = 3

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = 32 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 = sqlx::query_scalar(
        "SELECT id FROM runs WHERE name = 'evaluator-retry-twice-then-recover-e2e'",
    )
    .fetch_one(&harness.pool)
    .await?;

    harness.start_nodes(&["w-1", "w-2"]).await?;

    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-1",
            "sampler-aggregator",
            "evaluator-retry-twice-then-recover-e2e",
        ])
        .assert()
        .success();
    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-2",
            "evaluator",
            "evaluator-retry-twice-then-recover-e2e",
        ])
        .assert()
        .success();

    wait_for_batch_retry_count(&harness, run_id, 2, Duration::from_secs(60)).await?;
    wait_for_task_completed(&harness, run_id, Duration::from_secs(90)).await?;

    let failed_batches: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM batches WHERE run_id = $1 AND status = 'failed'")
            .bind(run_id)
            .fetch_one(&harness.pool)
            .await?;
    assert_eq!(
        failed_batches, 0,
        "task recovered, so no batch should be permanently failed"
    );

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_evaluator_batch_fails_three_times_and_task_fails() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config = temp_run_add_config(
        r#"
name = "evaluator-retry-three-then-fail-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0
fail_on_batch_nrs = [1, 2, 3]

[sampler_aggregator_runner_params.queue]
max_batch_retries = 3

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = 32 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 = sqlx::query_scalar(
        "SELECT id FROM runs WHERE name = 'evaluator-retry-three-then-fail-e2e'",
    )
    .fetch_one(&harness.pool)
    .await?;

    harness.start_nodes(&["w-1", "w-2"]).await?;

    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-1",
            "sampler-aggregator",
            "evaluator-retry-three-then-fail-e2e",
        ])
        .assert()
        .success();
    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-2",
            "evaluator",
            "evaluator-retry-three-then-fail-e2e",
        ])
        .assert()
        .success();

    wait_for_failed_batch(&harness, run_id, Duration::from_secs(60)).await?;
    wait_for_task_failed_and_run_unassigned(&harness, run_id, Duration::from_secs(90)).await?;

    let task: (String, Option<String>) = sqlx::query_as(
        "SELECT state, failure_reason FROM run_tasks WHERE run_id = $1 AND sequence_nr = 1",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(task.0, "failed");
    let reason = task.1.unwrap_or_default();
    assert!(
        reason.contains("3/3"),
        "expected retry progress in failure reason, got: {reason}"
    );

    let failed_batches: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM batches WHERE run_id = $1 AND status = 'failed'")
            .bind(run_id)
            .fetch_one(&harness.pool)
            .await?;
    assert!(
        failed_batches >= 1,
        "expected at least one permanently failed batch"
    );

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_evaluator_build_failure_fails_task_and_unassigns_run() -> anyhow::Result<()>
{
    let mut harness = FullStackHarness::new().await?;

    let config = temp_run_add_config(
        r#"
name = "evaluator-build-fail-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0
fail_on_build = true

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = 32 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 =
        sqlx::query_scalar("SELECT id FROM runs WHERE name = 'evaluator-build-fail-e2e'")
            .fetch_one(&harness.pool)
            .await?;

    harness.start_nodes(&["w-1", "w-2"]).await?;

    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-1",
            "sampler-aggregator",
            "evaluator-build-fail-e2e",
        ])
        .assert()
        .success();
    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-2",
            "evaluator",
            "evaluator-build-fail-e2e",
        ])
        .assert()
        .success();

    wait_for_task_failed_and_run_unassigned(&harness, run_id, Duration::from_secs(30)).await?;

    let task: (String, Option<String>) = sqlx::query_as(
        "SELECT state, failure_reason FROM run_tasks WHERE run_id = $1 AND sequence_nr = 1",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(task.0, "failed");
    let reason = task.1.unwrap_or_default();
    assert!(
        reason.contains("permanently failed to start"),
        "expected build failure reason, got: {reason}"
    );
    assert!(
        reason.contains("build failure"),
        "expected injected build failure message, got: {reason}"
    );

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_parameter_scan_creates_child_runs_and_collects_measurements()
-> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness
        .start_nodes(&["scan-parent", "scan-s1", "scan-e1"])
        .await?;

    let config = temp_run_add_config(
        r#"
name = "parameter-scan-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[evaluator_runner_params]
min_tick_time_ms = 50
db_pool_size = 1

[sampler_aggregator_runner_params]
min_tick_time_ms = 10
db_pool_size = 1

[[task_queue]]
name = "scan"
kind = "parameter_scan"
max_concurrent_runs = 1
trial_run_toml = """
name = "parameter-scan-child-$(scale:1)"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 16 }
measurement = { quantity = "central_value" }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"""

[task_queue.parameter]
name = "scale"
values = [1, 2, 3]

[task_queue.measurement]
source_task = "sample"
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 = sqlx::query_scalar("SELECT id FROM runs WHERE name = 'parameter-scan-e2e'")
        .fetch_one(&harness.pool)
        .await?;

    harness
        .cli()
        .args(["auto-assign", &run_id.to_string()])
        .assert()
        .success();

    if let Err(err) = harness
        .wait_for(
            "parameter scan completes",
            Duration::from_secs(90),
            || async {
                let state: Option<String> = sqlx::query_scalar(
                    "SELECT state FROM run_tasks WHERE run_id = $1 AND name = 'scan'",
                )
                .bind(run_id)
                .fetch_optional(&harness.pool)
                .await?;
                Ok(state.as_deref() == Some("completed"))
            },
        )
        .await
    {
        let runs: Vec<(i32, String, Option<i32>, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT id, name, parent_run_id, spawn_kind, spawn_label FROM runs ORDER BY id",
        )
        .fetch_all(&harness.pool)
        .await?;
        let tasks: Vec<(i32, String, String, Option<JsonValue>)> = sqlx::query_as(
            "SELECT run_id, name, state, measurement_output FROM run_tasks ORDER BY run_id, sequence_nr",
        )
        .fetch_all(&harness.pool)
        .await?;
        let nodes: Vec<(String, Option<i32>, Option<String>, Option<i32>, Option<String>)> =
            sqlx::query_as(
                "SELECT name, desired_run_id, desired_role, active_run_id, active_role FROM nodes ORDER BY name",
            )
            .fetch_all(&harness.pool)
            .await?;
        let logs: Vec<(String, String, JsonValue)> = sqlx::query_as(
            "SELECT level, message, fields FROM runtime_logs ORDER BY id DESC LIMIT 20",
        )
        .fetch_all(&harness.pool)
        .await?;
        anyhow::bail!("{err}; runs={runs:?}; tasks={tasks:?}; nodes={nodes:?}; logs={logs:?}");
    }

    let child_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runs WHERE parent_run_id = $1")
        .bind(run_id)
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(child_count, 3);

    let output: JsonValue = sqlx::query_scalar(
        "SELECT controller_output FROM run_tasks WHERE run_id = $1 AND name = 'scan'",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(output["completed_points"], json!(3));
    assert_eq!(output["total_points"], json!(3));
    assert_eq!(output["points"].as_array().map(Vec::len), Some(3));

    let measured_children: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM runs r
        JOIN run_tasks t ON t.run_id = r.id
        WHERE r.parent_run_id = $1
          AND r.spawn_kind = 'parameter_scan'
          AND t.name = 'sample'
          AND t.measurement_output IS NOT NULL
        "#,
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(measured_children, 3);

    let parent_assignments: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT name, desired_role FROM nodes WHERE desired_run_id = $1 ORDER BY name",
    )
    .bind(run_id)
    .fetch_all(&harness.pool)
    .await?;
    assert!(
        parent_assignments.is_empty(),
        "server-side scan controller should release parent compute assignments"
    );

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_parameter_scan_cartesian_product_parameters() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness
        .start_nodes(&["scan-grid-parent", "scan-grid-s1", "scan-grid-e1"])
        .await?;

    let config = temp_run_add_config(
        r#"
name = "parameter-scan-grid-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[evaluator_runner_params]
min_tick_time_ms = 50
db_pool_size = 1

[sampler_aggregator_runner_params]
min_tick_time_ms = 10
db_pool_size = 1

[[task_queue]]
name = "scan"
kind = "parameter_scan"
max_concurrent_runs = 2
trial_run_toml = """
name = "parameter-scan-grid-child-$(scale:1)-$(offset:0)"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 16 }
measurement = { quantity = "central_value" }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"""

[[task_queue.parameters]]
name = "scale"
values = [1, 2]

[[task_queue.parameters]]
name = "offset"
values = [0, 10]

[task_queue.measurement]
source_task = "sample"
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 =
        sqlx::query_scalar("SELECT id FROM runs WHERE name = 'parameter-scan-grid-e2e'")
            .fetch_one(&harness.pool)
            .await?;

    harness
        .cli()
        .args(["auto-assign", &run_id.to_string()])
        .assert()
        .success();

    harness
        .wait_for(
            "multi-parameter scan completes",
            Duration::from_secs(90),
            || async {
                let state: Option<String> = sqlx::query_scalar(
                    "SELECT state FROM run_tasks WHERE run_id = $1 AND name = 'scan'",
                )
                .bind(run_id)
                .fetch_optional(&harness.pool)
                .await?;
                Ok(state.as_deref() == Some("completed"))
            },
        )
        .await?;

    let output: JsonValue = sqlx::query_scalar(
        "SELECT controller_output FROM run_tasks WHERE run_id = $1 AND name = 'scan'",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(output["parameters"], json!(["scale", "offset"]));
    assert_eq!(output["completed_points"], json!(4));
    assert_eq!(output["total_points"], json!(4));
    let points = output["points"].as_array().expect("points");
    assert_eq!(points.len(), 4);
    assert_eq!(
        points[0]["parameter_values"],
        json!({"offset": 0, "scale": 1})
    );
    assert_eq!(
        points[1]["parameter_values"],
        json!({"offset": 10, "scale": 1})
    );
    assert_eq!(
        points[2]["parameter_values"],
        json!({"offset": 0, "scale": 2})
    );
    assert_eq!(
        points[3]["parameter_values"],
        json!({"offset": 10, "scale": 2})
    );

    let child_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM runs WHERE parent_run_id = $1 AND spawn_kind = 'parameter_scan'",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(child_count, 4);

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_parameter_scan_hands_over_to_next_scan_task() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness
        .start_nodes(&["scan-chain-parent", "scan-chain-s1", "scan-chain-e1"])
        .await?;

    let config = temp_run_add_config(
        r#"
name = "parameter-scan-chain-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[evaluator_runner_params]
min_tick_time_ms = 50
db_pool_size = 1

[sampler_aggregator_runner_params]
min_tick_time_ms = 10
db_pool_size = 1

[[task_queue]]
name = "scan-a"
kind = "parameter_scan"
max_concurrent_runs = 1
trial_run_toml = """
name = "parameter-scan-chain-a-$(scale:1)"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 16 }
measurement = { quantity = "central_value" }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"""

[task_queue.parameter]
name = "scale"
values = [1, 2]

[task_queue.measurement]
source_task = "sample"

[[task_queue]]
name = "scan-b"
kind = "parameter_scan"
max_concurrent_runs = 1
trial_run_toml = """
name = "parameter-scan-chain-b-$(offset:1)"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 16 }
measurement = { quantity = "central_value" }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"""

[task_queue.parameter]
name = "offset"
values = [10, 20]

[task_queue.measurement]
source_task = "sample"
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 =
        sqlx::query_scalar("SELECT id FROM runs WHERE name = 'parameter-scan-chain-e2e'")
            .fetch_one(&harness.pool)
            .await?;

    harness
        .cli()
        .args(["auto-assign", &run_id.to_string()])
        .assert()
        .success();

    if let Err(err) = harness
        .wait_for(
            "second parameter scan completes after first scan",
            Duration::from_secs(90),
            || async {
                let completed: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM run_tasks WHERE run_id = $1 AND name IN ('scan-a', 'scan-b') AND state = 'completed'",
                )
                .bind(run_id)
                .fetch_one(&harness.pool)
                .await?;
                Ok(completed == 2)
            },
        )
        .await
    {
        let tasks: Vec<(String, String, Option<JsonValue>)> = sqlx::query_as(
            "SELECT name, state, controller_output FROM run_tasks WHERE run_id = $1 ORDER BY sequence_nr",
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
        anyhow::bail!("{err}; tasks={tasks:?}; nodes={nodes:?}");
    }

    let child_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM runs WHERE parent_run_id = $1 AND spawn_kind = 'parameter_scan'",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(child_count, 4);

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_parameter_scan_redistributes_parent_assignments_and_updates_progress()
-> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness
        .start_nodes(&["scan-owned-s", "scan-owned-e1", "scan-owned-e2"])
        .await?;

    let config = temp_run_add_config(
        r#"
name = "parameter-scan-redistribute-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[evaluator_runner_params]
min_tick_time_ms = 50
db_pool_size = 1

[sampler_aggregator_runner_params]
min_tick_time_ms = 10
db_pool_size = 1

[[task_queue]]
name = "scan"
kind = "parameter_scan"
max_concurrent_runs = 2
trial_run_toml = """
name = "parameter-scan-redistribute-child-$(scale:1)"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 512 }
measurement = { quantity = "central_value" }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"""

[task_queue.parameter]
name = "scale"
values = [1, 2, 3, 4, 5]

[task_queue.measurement]
source_task = "sample"
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 =
        sqlx::query_scalar("SELECT id FROM runs WHERE name = 'parameter-scan-redistribute-e2e'")
            .fetch_one(&harness.pool)
            .await?;

    for (node, role) in [
        ("scan-owned-s", "sampler-aggregator"),
        ("scan-owned-e1", "evaluator"),
        ("scan-owned-e2", "evaluator"),
    ] {
        harness
            .cli()
            .args([
                "node",
                "assign",
                node,
                role,
                "parameter-scan-redistribute-e2e",
            ])
            .assert()
            .success();
    }

    harness
        .wait_for(
            "scan redistributes parent assignments to child runs",
            Duration::from_secs(30),
            || async {
                let parent_desired: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM nodes WHERE desired_run_id = $1")
                        .bind(run_id)
                        .fetch_one(&harness.pool)
                        .await?;
                let child_desired: i64 = sqlx::query_scalar(
                    r#"
                    SELECT COUNT(*)
                    FROM nodes n
                    JOIN runs r ON r.id = n.desired_run_id
                    WHERE r.parent_run_id = $1
                      AND r.spawn_kind = 'parameter_scan'
                    "#,
                )
                .bind(run_id)
                .fetch_one(&harness.pool)
                .await?;
                Ok(parent_desired == 0 && child_desired > 0)
            },
        )
        .await?;

    harness
        .wait_for(
            "scan controller progress updates before completion",
            Duration::from_secs(60),
            || async {
                let output: Option<JsonValue> = sqlx::query_scalar::<_, Option<JsonValue>>(
                    "SELECT controller_output FROM run_tasks WHERE run_id = $1 AND name = 'scan'",
                )
                .bind(run_id)
                .fetch_optional(&harness.pool)
                .await?
                .flatten();
                let completed = output
                    .as_ref()
                    .and_then(|value| value.get("completed_points"))
                    .and_then(JsonValue::as_i64)
                    .unwrap_or(0);
                let total = output
                    .as_ref()
                    .and_then(|value| value.get("total_points"))
                    .and_then(JsonValue::as_i64)
                    .unwrap_or(0);
                Ok(completed > 0 && completed < total)
            },
        )
        .await?;

    harness
        .wait_for(
            "redistribution scan completes",
            Duration::from_secs(90),
            || async {
                let state: Option<String> = sqlx::query_scalar(
                    "SELECT state FROM run_tasks WHERE run_id = $1 AND name = 'scan'",
                )
                .bind(run_id)
                .fetch_optional(&harness.pool)
                .await?;
                Ok(state.as_deref() == Some("completed"))
            },
        )
        .await?;

    let output: JsonValue = sqlx::query_scalar(
        "SELECT controller_output FROM run_tasks WHERE run_id = $1 AND name = 'scan'",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(output["completed_points"], json!(5));
    assert_eq!(output["total_points"], json!(5));

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_hyperparameter_tuning_random_search_creates_trials_and_collects_objectives()
-> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness
        .start_nodes(&["tune-parent", "tune-s1", "tune-e1"])
        .await?;

    let config = temp_run_add_config(
        r#"
name = "hyperparameter-tuning-random-e2e"

[evaluator_runner_params]
min_tick_time_ms = 50
db_pool_size = 1

[sampler_aggregator_runner_params]
min_tick_time_ms = 10
db_pool_size = 1

[[task_queue]]
name = "tune"
kind = "hyperparameter_tuning"
max_concurrent_trials = 2
trial_run_toml = """
name = "tuning-child-a-$(a:0.0)-bins-$(bins:16)-mode-$(mode:auto)"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 16 }
measurement = { quantity = "central_value" }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"""

[task_queue.optimizer]
algorithm = "random_search"

[task_queue.optimizer.params]
max_trials = 4
seed = 3

[task_queue.objective]
source_task = "sample"
mode = "minimize"
quantity = "central_value"

[task_queue.parameters.a]
kind = "float"
min = 0.0
max = 1.0

[task_queue.parameters.bins]
kind = "integer"
min = 8
max = 16
step = 4

[task_queue.parameters.mode]
kind = "categorical"
values = ["auto", "none"]
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 =
        sqlx::query_scalar("SELECT id FROM runs WHERE name = 'hyperparameter-tuning-random-e2e'")
            .fetch_one(&harness.pool)
            .await?;

    harness
        .cli()
        .args(["auto-assign", &run_id.to_string()])
        .assert()
        .success();

    harness
        .wait_for(
            "random tuning completes",
            Duration::from_secs(90),
            || async {
                let state: Option<String> = sqlx::query_scalar(
                    "SELECT state FROM run_tasks WHERE run_id = $1 AND name = 'tune'",
                )
                .bind(run_id)
                .fetch_optional(&harness.pool)
                .await?;
                Ok(state.as_deref() == Some("completed"))
            },
        )
        .await?;

    let child_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM runs WHERE parent_run_id = $1 AND spawn_kind = 'hyperparameter_tuning'",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(child_count, 4);

    let output: JsonValue = sqlx::query_scalar(
        "SELECT controller_output FROM run_tasks WHERE run_id = $1 AND name = 'tune'",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(output["completed_trials"], json!(4));
    assert_eq!(output["failed_trials"], json!(0));
    assert_eq!(output["total_trials"], json!(4));
    assert_eq!(output["trials"].as_array().map(Vec::len), Some(4));
    assert!(output["best_trial"].is_number());

    let measured_children: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM runs r
        JOIN run_tasks t ON t.run_id = r.id
        WHERE r.parent_run_id = $1
          AND r.spawn_kind = 'hyperparameter_tuning'
          AND t.name = 'sample'
          AND t.measurement_output IS NOT NULL
        "#,
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(measured_children, 4);

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_hyperparameter_tuning_egobox_creates_adaptive_trials() -> anyhow::Result<()>
{
    let mut harness = FullStackHarness::new().await?;
    harness
        .start_nodes(&["egobox-tune-parent", "egobox-tune-s1", "egobox-tune-e1"])
        .await?;

    let config = temp_run_add_config(
        r#"
name = "hyperparameter-tuning-egobox-e2e"

[evaluator_runner_params]
min_tick_time_ms = 50
db_pool_size = 1

[sampler_aggregator_runner_params]
min_tick_time_ms = 10
db_pool_size = 1

[[task_queue]]
name = "tune"
kind = "hyperparameter_tuning"
max_concurrent_trials = 1
trial_run_toml = """
name = "egobox-child-bins-$(bins:1)"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 16 }
measurement = { quantity = "central_value" }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"""

[task_queue.optimizer]
algorithm = "egobox"

[task_queue.optimizer.params]
max_trials = 3
seed = 3
initial_design = 2
parallel_candidates = 1
infill = "ei"

[task_queue.objective]
source_task = "sample"
mode = "minimize"
quantity = "central_value"

[task_queue.parameters.bins]
kind = "integer"
min = 1
max = 4
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 =
        sqlx::query_scalar("SELECT id FROM runs WHERE name = 'hyperparameter-tuning-egobox-e2e'")
            .fetch_one(&harness.pool)
            .await?;

    harness
        .cli()
        .args(["auto-assign", &run_id.to_string()])
        .assert()
        .success();

    if let Err(err) = harness
        .wait_for(
            "egobox tuning completes",
            Duration::from_secs(90),
            || async {
                let state: Option<String> = sqlx::query_scalar(
                    "SELECT state FROM run_tasks WHERE run_id = $1 AND name = 'tune'",
                )
                .bind(run_id)
                .fetch_optional(&harness.pool)
                .await?;
                Ok(state.as_deref() == Some("completed"))
            },
        )
        .await
    {
        let runs: Vec<(i32, String, Option<i32>, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT id, name, parent_run_id, spawn_kind, spawn_label FROM runs ORDER BY id",
        )
        .fetch_all(&harness.pool)
        .await?;
        let tasks: Vec<(i32, String, String, Option<JsonValue>, Option<String>)> =
            sqlx::query_as(
                "SELECT run_id, name, state, controller_output, failure_reason FROM run_tasks ORDER BY run_id, sequence_nr",
            )
            .fetch_all(&harness.pool)
            .await?;
        let nodes: Vec<(String, Option<i32>, Option<String>, Option<i32>, Option<String>)> =
            sqlx::query_as(
                "SELECT name, desired_run_id, desired_role, active_run_id, active_role FROM nodes ORDER BY name",
            )
            .fetch_all(&harness.pool)
            .await?;
        let logs: Vec<(String, String, JsonValue)> = sqlx::query_as(
            "SELECT level, message, fields FROM runtime_logs ORDER BY id DESC LIMIT 20",
        )
        .fetch_all(&harness.pool)
        .await?;
        anyhow::bail!("{err}; runs={runs:?}; tasks={tasks:?}; nodes={nodes:?}; logs={logs:?}");
    }

    let output: JsonValue = sqlx::query_scalar(
        "SELECT controller_output FROM run_tasks WHERE run_id = $1 AND name = 'tune'",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(output["completed_trials"], json!(3));
    assert_eq!(output["failed_trials"], json!(0));
    assert_eq!(output["total_trials"], json!(3));
    let trials = output["trials"].as_array().expect("trials");
    assert_eq!(trials.len(), 3);
    for trial in trials {
        assert!(trial["parameters"]["bins"].is_i64());
        assert!(trial["objective_value"].is_number());
    }

    let child_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM runs WHERE parent_run_id = $1 AND spawn_kind = 'hyperparameter_tuning'",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(child_count, 3);

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_hyperparameter_tuning_grid_search_enumerates_finite_domains()
-> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness
        .start_nodes(&["grid-tune-parent", "grid-tune-s1", "grid-tune-e1"])
        .await?;

    let config = temp_run_add_config(
        r#"
name = "hyperparameter-tuning-grid-e2e"

[evaluator_runner_params]
min_tick_time_ms = 50
db_pool_size = 1

[sampler_aggregator_runner_params]
min_tick_time_ms = 10
db_pool_size = 1

[[task_queue]]
name = "tune"
kind = "hyperparameter_tuning"
max_concurrent_trials = 2
trial_run_toml = """
name = "grid-child-bins-$(bins:8)-mode-$(mode:auto)"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 16 }
measurement = { quantity = "central_value" }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"""

[task_queue.optimizer]
algorithm = "grid_search"

[task_queue.objective]
source_task = "sample"
mode = "minimize"
quantity = "central_value"

[task_queue.parameters.bins]
kind = "integer"
min = 8
max = 16
step = 4

[task_queue.parameters.mode]
kind = "categorical"
values = ["auto", "none"]
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 =
        sqlx::query_scalar("SELECT id FROM runs WHERE name = 'hyperparameter-tuning-grid-e2e'")
            .fetch_one(&harness.pool)
            .await?;

    harness
        .cli()
        .args(["auto-assign", &run_id.to_string()])
        .assert()
        .success();

    harness
        .wait_for("grid tuning completes", Duration::from_secs(90), || async {
            let state: Option<String> = sqlx::query_scalar(
                "SELECT state FROM run_tasks WHERE run_id = $1 AND name = 'tune'",
            )
            .bind(run_id)
            .fetch_optional(&harness.pool)
            .await?;
            Ok(state.as_deref() == Some("completed"))
        })
        .await?;

    let output: JsonValue = sqlx::query_scalar(
        "SELECT controller_output FROM run_tasks WHERE run_id = $1 AND name = 'tune'",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(output["completed_trials"], json!(6));
    assert_eq!(output["failed_trials"], json!(0));
    assert_eq!(output["total_trials"], json!(6));
    let trials = output["trials"].as_array().expect("trials");
    assert_eq!(trials.len(), 6);
    assert_eq!(trials[0]["parameters"], json!({"bins": 8, "mode": "auto"}));
    assert_eq!(trials[1]["parameters"], json!({"bins": 8, "mode": "none"}));
    assert_eq!(trials[5]["parameters"], json!({"bins": 16, "mode": "none"}));

    let child_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM runs WHERE parent_run_id = $1 AND spawn_kind = 'hyperparameter_tuning'",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(child_count, 6);

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_hyperparameter_tuning_fails_on_bad_trial_template() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness
        .start_nodes(&["bad-tune-parent", "bad-tune-s1", "bad-tune-e1"])
        .await?;

    let config = temp_run_add_config(
        r#"
name = "hyperparameter-tuning-bad-template-e2e"

[evaluator_runner_params]
min_tick_time_ms = 50
db_pool_size = 1

[sampler_aggregator_runner_params]
min_tick_time_ms = 10
db_pool_size = 1

[[task_queue]]
name = "tune"
kind = "hyperparameter_tuning"
max_concurrent_trials = 1
trial_run_toml = "name = \"broken-child"

[task_queue.optimizer]
algorithm = "random_search"

[task_queue.optimizer.params]
max_trials = 1
seed = 3

[task_queue.objective]
source_task = "sample"
mode = "minimize"
quantity = "central_value"

[task_queue.parameters.a]
kind = "float"
min = 0.0
max = 1.0
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 = sqlx::query_scalar(
        "SELECT id FROM runs WHERE name = 'hyperparameter-tuning-bad-template-e2e'",
    )
    .fetch_one(&harness.pool)
    .await?;

    harness
        .cli()
        .args(["auto-assign", &run_id.to_string()])
        .assert()
        .success();

    harness
        .wait_for(
            "bad template tuning fails",
            Duration::from_secs(60),
            || async {
                let state: Option<String> = sqlx::query_scalar(
                    "SELECT state FROM run_tasks WHERE run_id = $1 AND name = 'tune'",
                )
                .bind(run_id)
                .fetch_optional(&harness.pool)
                .await?;
                Ok(state.as_deref() == Some("failed"))
            },
        )
        .await?;

    let failure_reason: String = sqlx::query_scalar(
        "SELECT failure_reason FROM run_tasks WHERE run_id = $1 AND name = 'tune'",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert!(failure_reason.contains("failed to create tuning trial 0"));

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_hyperparameter_tuning_fails_when_child_measurement_fails()
-> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness
        .start_nodes(&[
            "failed-measure-parent",
            "failed-measure-s1",
            "failed-measure-e1",
        ])
        .await?;

    let config = temp_run_add_config(
        r#"
name = "hyperparameter-tuning-failed-measurement-e2e"

[evaluator_runner_params]
min_tick_time_ms = 50
db_pool_size = 1

[sampler_aggregator_runner_params]
min_tick_time_ms = 10
db_pool_size = 1

[[task_queue]]
name = "tune"
kind = "hyperparameter_tuning"
max_concurrent_trials = 1
trial_run_toml = """
name = "failed-measure-child-$(scale:1)"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 16 }
measurement = { quantity = { metric = "variance", component = "missing" } }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"""

[task_queue.optimizer]
algorithm = "random_search"

[task_queue.optimizer.params]
max_trials = 1
seed = 5

[task_queue.objective]
source_task = "sample"
mode = "minimize"
quantity = "central_value"

[task_queue.parameters.scale]
kind = "integer"
min = 1
max = 1
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 = sqlx::query_scalar(
        "SELECT id FROM runs WHERE name = 'hyperparameter-tuning-failed-measurement-e2e'",
    )
    .fetch_one(&harness.pool)
    .await?;

    harness
        .cli()
        .args(["auto-assign", &run_id.to_string()])
        .assert()
        .success();

    harness
        .wait_for(
            "failed measurement tuning fails",
            Duration::from_secs(60),
            || async {
                let state: Option<String> = sqlx::query_scalar(
                    "SELECT state FROM run_tasks WHERE run_id = $1 AND name = 'tune'",
                )
                .bind(run_id)
                .fetch_optional(&harness.pool)
                .await?;
                Ok(state.as_deref() == Some("failed"))
            },
        )
        .await?;

    let output: JsonValue = sqlx::query_scalar(
        "SELECT controller_output FROM run_tasks WHERE run_id = $1 AND name = 'tune'",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(output["failed_trials"], json!(1));
    assert_eq!(output["completed_trials"], json!(0));
    assert!(
        output["trials"][0]["failure_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("child_run_id=")
                && reason.contains("source_task=sample")
                && reason.contains("requested=Mean")
                && reason.contains("Variance(component=missing)")
                && reason.contains("measurement failed")
                && reason.contains("unavailable"))
    );

    let failure_reason: String = sqlx::query_scalar(
        "SELECT failure_reason FROM run_tasks WHERE run_id = $1 AND name = 'tune'",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert!(failure_reason.contains("failed_trials=1"));

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_hyperparameter_tuning_redistributes_parent_assignments()
-> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness
        .start_nodes(&["tune-owned-s", "tune-owned-e1", "tune-owned-e2"])
        .await?;

    let config = temp_run_add_config(
        r#"
name = "hyperparameter-tuning-redistribute-e2e"

[evaluator_runner_params]
min_tick_time_ms = 50
db_pool_size = 1

[sampler_aggregator_runner_params]
min_tick_time_ms = 10
db_pool_size = 1

[[task_queue]]
name = "tune"
kind = "hyperparameter_tuning"
max_concurrent_trials = 2
trial_run_toml = """
name = "tune-redistribute-child-$(scale:1)"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 512 }
measurement = { quantity = "central_value" }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"""

[task_queue.optimizer]
algorithm = "grid_search"

[task_queue.objective]
source_task = "sample"
mode = "minimize"
quantity = "central_value"

[task_queue.parameters.scale]
kind = "integer"
min = 1
max = 4
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let run_id: i32 = sqlx::query_scalar(
        "SELECT id FROM runs WHERE name = 'hyperparameter-tuning-redistribute-e2e'",
    )
    .fetch_one(&harness.pool)
    .await?;

    for (node, role) in [
        ("tune-owned-s", "sampler-aggregator"),
        ("tune-owned-e1", "evaluator"),
        ("tune-owned-e2", "evaluator"),
    ] {
        harness
            .cli()
            .args([
                "node",
                "assign",
                node,
                role,
                "hyperparameter-tuning-redistribute-e2e",
            ])
            .assert()
            .success();
    }

    harness
        .wait_for(
            "tuning redistributes parent assignments to child runs",
            Duration::from_secs(30),
            || async {
                let parent_desired: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM nodes WHERE desired_run_id = $1")
                        .bind(run_id)
                        .fetch_one(&harness.pool)
                        .await?;
                let child_desired: i64 = sqlx::query_scalar(
                    r#"
                    SELECT COUNT(*)
                    FROM nodes n
                    JOIN runs r ON r.id = n.desired_run_id
                    WHERE r.parent_run_id = $1
                      AND r.spawn_kind = 'hyperparameter_tuning'
                    "#,
                )
                .bind(run_id)
                .fetch_one(&harness.pool)
                .await?;
                Ok(parent_desired == 0 && child_desired > 0)
            },
        )
        .await?;

    if let Err(err) = harness
        .wait_for(
            "redistribution tuning completes",
            Duration::from_secs(90),
            || async {
                let state: Option<String> = sqlx::query_scalar(
                    "SELECT state FROM run_tasks WHERE run_id = $1 AND name = 'tune'",
                )
                .bind(run_id)
                .fetch_optional(&harness.pool)
                .await?;
                Ok(state.as_deref() == Some("completed"))
            },
        )
        .await
    {
        let runs: Vec<(i32, String, Option<i32>, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT id, name, parent_run_id, spawn_kind, spawn_label FROM runs ORDER BY id",
        )
        .fetch_all(&harness.pool)
        .await?;
        let tasks: Vec<(i32, String, String, Option<JsonValue>, Option<String>)> =
            sqlx::query_as(
                "SELECT run_id, name, state, controller_output, failure_reason FROM run_tasks ORDER BY run_id, sequence_nr",
            )
            .fetch_all(&harness.pool)
            .await?;
        let nodes: Vec<(String, Option<i32>, Option<String>, Option<i32>, Option<String>)> =
            sqlx::query_as(
                "SELECT name, desired_run_id, desired_role, active_run_id, active_role FROM nodes ORDER BY name",
            )
            .fetch_all(&harness.pool)
            .await?;
        let logs: Vec<(String, String, JsonValue)> = sqlx::query_as(
            "SELECT level, message, fields FROM runtime_logs ORDER BY id DESC LIMIT 20",
        )
        .fetch_all(&harness.pool)
        .await?;
        anyhow::bail!("{err}; runs={runs:?}; tasks={tasks:?}; nodes={nodes:?}; logs={logs:?}");
    }

    let output: JsonValue = sqlx::query_scalar(
        "SELECT controller_output FROM run_tasks WHERE run_id = $1 AND name = 'tune'",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(output["completed_trials"], json!(4));
    assert_eq!(output["total_trials"], json!(4));

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_can_clone_run_from_task_snapshot() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config = temp_run_add_config(
        r#"
name = "clone-source-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = 16 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = 16 }
"#,
    );

    harness
        .cli()
        .arg("run")
        .arg("add")
        .arg(config.path())
        .assert()
        .success();

    let source_run_id: i32 =
        sqlx::query_scalar("SELECT id FROM runs WHERE name = 'clone-source-e2e'")
            .fetch_one(&harness.pool)
            .await?;
    let source_task_1: i64 =
        sqlx::query_scalar("SELECT id FROM run_tasks WHERE run_id = $1 AND sequence_nr = 1")
            .bind(source_run_id)
            .fetch_one(&harness.pool)
            .await?;

    harness.start_nodes(&["w-1", "w-2"]).await?;

    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-1",
            "sampler-aggregator",
            "clone-source-e2e",
        ])
        .assert()
        .success();
    harness
        .cli()
        .args(["node", "assign", "w-2", "evaluator", "clone-source-e2e"])
        .assert()
        .success();

    harness
        .wait_for("source run completes", Duration::from_secs(20), || async {
            let w1 = harness.node_state("w-1").await?;
            let w2 = harness.node_state("w-2").await?;
            let completed: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM run_tasks WHERE run_id = $1 AND state = 'completed'",
            )
            .bind(source_run_id)
            .fetch_one(&harness.pool)
            .await?;
            Ok(w1.0.is_none()
                && w1.1.is_none()
                && w1.2.is_none()
                && w1.3.is_none()
                && w2.0.is_none()
                && w2.1.is_none()
                && w2.2.is_none()
                && w2.3.is_none()
                && completed == 2)
        })
        .await?;

    let source_snapshot_id: i64 = sqlx::query_scalar(
        "SELECT id FROM run_stage_snapshots WHERE run_id = $1 AND task_id = $2 AND queue_empty = TRUE ORDER BY id DESC LIMIT 1",
    )
    .bind(source_run_id)
    .bind(source_task_1)
    .fetch_one(&harness.pool)
    .await?;

    harness
        .cli()
        .args([
            "run",
            "clone",
            "clone-source-e2e",
            &source_snapshot_id.to_string(),
            "clone-branch-e2e",
        ])
        .assert()
        .success();

    let cloned_run_id: i32 =
        sqlx::query_scalar("SELECT id FROM runs WHERE name = 'clone-branch-e2e'")
            .fetch_one(&harness.pool)
            .await?;
    let cloned_task_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM run_tasks WHERE run_id = $1")
            .bind(cloned_run_id)
            .fetch_one(&harness.pool)
            .await?;
    assert_eq!(cloned_task_count, 0);

    let cloned_root_snapshot_name: String = sqlx::query_scalar(
        "SELECT name FROM run_stage_snapshots WHERE run_id = $1 AND task_id IS NULL ORDER BY id ASC LIMIT 1",
    )
    .bind(cloned_run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert!(
        cloned_root_snapshot_name.contains("clone_of:clone-source-e2e"),
        "unexpected cloned root snapshot name: {cloned_root_snapshot_name}"
    );

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}
