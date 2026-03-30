use argon2::{
    Argon2,
    password_hash::{PasswordHasher, SaltString, rand_core::OsRng},
};
use assert_cmd::Command;
use gammaboard::config::CliConfig;
use predicates::prelude::*;
use serde_json::json;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::NamedTempFile;
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
        let base_url = CliConfig::load("configs/cli/default.toml")?.database.url;

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
    cli_config_path: PathBuf,
    temp_files: Vec<NamedTempFile>,
}

struct ManagedChild {
    label: String,
    child: Child,
}

impl FullStackHarness {
    async fn new() -> anyhow::Result<Self> {
        let db = TestDatabase::create().await?;
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&db.database_url)
            .await?;
        let bin_path = resolve_bin_path()?;
        let cli_config = temp_cli_config(&db.database_url, false);
        let cli_config_path = cli_config.path().to_path_buf();

        let mut temp_files = Vec::new();
        temp_files.push(cli_config);

        Ok(Self {
            db,
            pool,
            bin_path,
            children: Vec::new(),
            cli_config_path,
            temp_files,
        })
    }

    fn cli(&self) -> Command {
        let mut cmd = Command::new(&self.bin_path);
        cmd.arg("--cli-config").arg(&self.cli_config_path);
        cmd
    }

    async fn start_node(&mut self, node_name: &str) -> anyhow::Result<()> {
        let mut child = TokioCommand::new(&self.bin_path);
        child
            .arg("--cli-config")
            .arg(&self.cli_config_path)
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
                    let count: i64 =
                        sqlx::query_scalar("SELECT COUNT(*) FROM nodes WHERE name = $1")
                            .bind(&node_name)
                            .fetch_one(&pool)
                            .await?;
                    Ok(count == 1)
                }
            },
        )
        .await
    }

    async fn start_server(&mut self) -> anyhow::Result<String> {
        let password_hash = hash_password_for_tests("test-password");
        self.start_server_with_auth((&password_hash, "test-session-secret"))
            .await
    }

    async fn start_server_with_auth(&mut self, auth: (&str, &str)) -> anyhow::Result<String> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let addr = listener.local_addr()?;
        drop(listener);
        let server_config = temp_server_config(
            &addr.ip().to_string(),
            addr.port(),
            "http://localhost:3000",
            false,
            auth,
        );

        let mut child = TokioCommand::new(&self.bin_path);
        child
            .arg("--cli-config")
            .arg(&self.cli_config_path)
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

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_alternating_havana_e2e() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    // Initial run with tasks 1..4:
    // 1: havana_training
    // 2: havana_inference
    // 3: naive_monte_carlo
    // 4: image
    let config = temp_run_config(
        r#"
name = "havana-alt-e2e"

[evaluator]
kind = "sinc_evaluator"

[[task_queue]]
name = "train-a"
kind = "sample"
nr_samples = 128
observable = { config = "complex" }
sampler_aggregator = { config = { kind = "havana_training", seed = 0, bins = 8, min_samples_for_update = 4, samples_for_update = 8 } }

[[task_queue]]
name = "infer-a"
kind = "sample"
nr_samples = 128
sampler_aggregator = { config = { kind = "havana_inference" } }

[[task_queue]]
name = "naive-a"
kind = "sample"
nr_samples = 32
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }

[[task_queue]]
kind = "image"
observable = "complex"
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
    harness.start_node("w-1").await?;
    harness.start_node("w-2").await?;

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
nr_samples = 128
sampler_aggregator = { from_name = "infer-a" }
observable = { from_name = "infer-a" }

[[task_queue]]
kind = "sample"
nr_samples = 128
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
                Ok(completed == 7)
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

fn temp_server_config(
    host: &str,
    port: u16,
    allowed_origin: &str,
    secure_cookie: bool,
    auth: (&str, &str),
) -> NamedTempFile {
    let (admin_password_hash, session_secret) = auth;
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let run_templates_dir = manifest_dir.join("configs/runs");
    let task_templates_dir = manifest_dir.join("configs/tasks");
    let contents = format!(
        "host = {host:?}\nport = {port}\nallowed_origin = {allowed_origin:?}\nsecure_cookie = {secure_cookie}\nallow_db_admin = true\nrun_templates_dir = {run_templates_dir:?}\ntask_templates_dir = {task_templates_dir:?}\n\n[auth]\nadmin_password_hash = {admin_password_hash:?}\nsession_secret = {session_secret:?}\n"
    );
    let file = NamedTempFile::new().expect("create temp server config");
    std::fs::write(file.path(), contents).expect("write temp server config");
    file
}

fn temp_cli_config(database_url: &str, persist_runtime_logs: bool) -> NamedTempFile {
    let contents = format!(
        "[database]\nurl = {database_url:?}\n\n[tracing]\npersist_runtime_logs = {persist_runtime_logs}\ndb_gammaboard_level = \"info\"\ndb_external_level = \"warn\"\n\n[local_postgres]\ndata_dir = \".postgres\"\nsocket_dir = \".postgres-socket\"\nlog_file = \".postgres/logfile\"\n"
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

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_flow_exercises_run_and_node_lifecycle() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let invalid_config = temp_run_config(
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

    let valid_config = temp_run_config(
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

    harness.start_node("w-1").await?;
    harness.start_node("w-2").await?;

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
        .wait_for("two active evaluators", Duration::from_secs(10), || async {
            let w1 = harness.node_state("w-1").await?;
            let w2 = harness.node_state("w-2").await?;
            Ok(w1.0 == Some(run_id)
                && w1.1.as_deref() == Some("evaluator")
                && w1.2 == Some(run_id)
                && w1.3.as_deref() == Some("evaluator")
                && w2.0 == Some(run_id)
                && w2.1.as_deref() == Some("evaluator")
                && w2.2 == Some(run_id)
                && w2.3.as_deref() == Some("evaluator"))
        })
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
            "reassigned evaluators become active",
            Duration::from_secs(10),
            || async {
                let w1 = harness.node_state("w-1").await?;
                let w2 = harness.node_state("w-2").await?;
                Ok(w1.0 == Some(run_id)
                    && w1.1.as_deref() == Some("evaluator")
                    && w1.2 == Some(run_id)
                    && w1.3.as_deref() == Some("evaluator")
                    && w2.0 == Some(run_id)
                    && w2.1.as_deref() == Some("evaluator")
                    && w2.2 == Some(run_id)
                    && w2.3.as_deref() == Some("evaluator"))
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
            "resumed run becomes active again",
            Duration::from_secs(10),
            || async {
                let w1 = harness.node_state("w-1").await?;
                let w2 = harness.node_state("w-2").await?;
                Ok(w1.2 == Some(run_id)
                    && w1.3.as_deref() == Some("evaluator")
                    && w2.2 == Some(run_id)
                    && w2.3.as_deref() == Some("evaluator"))
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
async fn full_stack_cli_server_can_restart_while_nodes_keep_running() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let server_url = harness.start_server().await?;
    harness.start_node("w-1").await?;
    harness.start_node("w-2").await?;

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
    harness.start_node("w-1").await?;

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

    harness.start_node("w-1").await?;

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_server_auth_protects_pause_endpoint() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config = temp_run_config("name = \"auth-e2e\"\n");
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

    let shutdown_requested_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT shutdown_requested_at FROM nodes WHERE name = 'w-1'")
            .fetch_one(&harness.pool)
            .await?;
    assert!(shutdown_requested_at.is_some());

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_lists_duplicate_run_names_and_reports_ambiguity() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config_a = temp_run_config("name = \"duplicate-run\"\n");
    let config_b = temp_run_config("name = \"duplicate-run\"\n");

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

    let config = temp_run_config(
        r#"
name = "worker-death-e2e"

[evaluator]
kind = "sin_evaluator"
min_eval_time_per_sample_ms = 20

[[task_queue]]
kind = "sample"
nr_samples = 128
observable = "scalar"
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }

[evaluator_runner_params]
performance_snapshot_interval_ms = 200

[sampler_aggregator_runner_params]
performance_snapshot_interval_ms = 200
target_batch_eval_ms = 250.0
target_queue_remaining = 0.5
max_batch_size = 16
max_batches_per_tick = 4
max_queue_size = 32
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

    let run_id: i32 = sqlx::query_scalar("SELECT id FROM runs WHERE name = 'worker-death-e2e'")
        .fetch_one(&harness.pool)
        .await?;

    harness.start_node("w-1").await?;
    harness.start_node("w-2").await?;
    harness.start_node("w-3").await?;

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

    let config = temp_run_config(
        r#"
name = "sampler-error-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0
observable_kind = "scalar"

[[task_queue]]
kind = "sample"
nr_samples = 32
observable = "scalar"
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

    harness.start_node("w-1").await?;
    harness.start_node("w-2").await?;

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
async fn full_stack_cli_fails_task_gracefully_on_materializer_error() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config = temp_run_config(
        r#"
name = "materializer-error-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0
observable_kind = "scalar"

[[task_queue]]
kind = "sample"
nr_samples = 32
observable = "scalar"
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

    harness.start_node("w-1").await?;
    harness.start_node("w-2").await?;

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

    wait_for_task_failed_and_run_unassigned(&harness, run_id, Duration::from_secs(40)).await?;

    let failed_batches: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM batches WHERE run_id = $1 AND status = 'failed'")
            .bind(run_id)
            .fetch_one(&harness.pool)
            .await?;
    assert!(
        failed_batches > 0,
        "expected failed batches for materializer error"
    );

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_fails_task_gracefully_on_evaluator_error() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config = temp_run_config(
        r#"
name = "evaluator-error-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0
observable_kind = "scalar"
fail_on_batch_nr = 1

[[task_queue]]
kind = "sample"
nr_samples = 32
observable = "scalar"
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

    harness.start_node("w-1").await?;
    harness.start_node("w-2").await?;

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

    wait_for_task_failed_and_run_unassigned(&harness, run_id, Duration::from_secs(40)).await?;

    let failed_batches: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM batches WHERE run_id = $1 AND status = 'failed'")
            .bind(run_id)
            .fetch_one(&harness.pool)
            .await?;
    assert!(
        failed_batches > 0,
        "expected failed batches for evaluator error"
    );

    harness.stop_children().await;
    harness.pool.close().await;
    harness.db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_can_clone_run_from_task_snapshot() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config = temp_run_config(
        r#"
name = "clone-source-e2e"

[evaluator]
kind = "sin_evaluator"

[[task_queue]]
kind = "sample"
nr_samples = 16
observable = "scalar"
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }

[[task_queue]]
kind = "sample"
nr_samples = 16
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

    harness.start_node("w-1").await?;
    harness.start_node("w-2").await?;

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
                && completed == 3)
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
    assert_eq!(cloned_task_count, 1);

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
