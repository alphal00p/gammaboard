use assert_cmd::Command;
use predicates::prelude::*;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::NamedTempFile;
use tokio::process::{Child, Command as TokioCommand};
use tokio::time::{Instant, sleep};
use url::Url;

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_nanos();
    nanos.to_string()
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
        let base_url = std::env::var("DATABASE_URL")
            .map_err(|_| anyhow::anyhow!("DATABASE_URL must be set for full-stack tests"))?;

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
    children: Vec<Child>,
}

impl FullStackHarness {
    async fn new() -> anyhow::Result<Self> {
        let db = TestDatabase::create().await?;
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&db.database_url)
            .await?;
        let bin_path = resolve_bin_path()?;

        Ok(Self {
            db,
            pool,
            bin_path,
            children: Vec::new(),
        })
    }

    fn cli(&self) -> Command {
        let mut cmd = Command::new(&self.bin_path);
        cmd.env("DATABASE_URL", &self.db.database_url);
        cmd.env("GAMMABOARD_DISABLE_DB_LOGS", "1");
        cmd
    }

    async fn start_node(&mut self, node_name: &str) -> anyhow::Result<()> {
        let mut child = TokioCommand::new(&self.bin_path);
        child
            .env("DATABASE_URL", &self.db.database_url)
            .env("GAMMABOARD_DISABLE_DB_LOGS", "1")
            .arg("run-node")
            .arg("--name")
            .arg(node_name)
            .arg("--poll-ms")
            .arg("100")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        let child = child.spawn()?;
        self.children.push(child);

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
        for child in &mut self.children {
            let _ = child.start_kill();
        }
        for child in &mut self.children {
            let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
        }
        self.children.clear();
    }
}

impl Drop for FullStackHarness {
    fn drop(&mut self) {
        for child in &mut self.children {
            let _ = child.start_kill();
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
            "top-level [point_spec] is no longer supported",
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
        .args(["node", "assign", "w-1", "evaluator", &run_id.to_string()])
        .assert()
        .success();
    harness
        .cli()
        .args(["node", "assign", "w-2", "evaluator", &run_id.to_string()])
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
            &run_id.to_string(),
        ])
        .assert()
        .success();

    let ghost_state = harness.node_state("ghost-node").await?;
    assert_eq!(ghost_state.0, Some(run_id));
    assert_eq!(ghost_state.1.as_deref(), Some("evaluator"));
    assert_eq!(ghost_state.2, None);
    assert_eq!(ghost_state.3, None);

    harness
        .cli()
        .args([
            "node",
            "assign",
            "w-2",
            "sampler-aggregator",
            &run_id.to_string(),
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
                let ghost = harness.node_state("ghost-node").await?;
                Ok(w1.0.is_none()
                    && w1.1.is_none()
                    && w1.2.is_none()
                    && w1.3.is_none()
                    && w2.0.is_none()
                    && w2.1.is_none()
                    && w2.2.is_none()
                    && w2.3.is_none()
                    && ghost.0.is_none()
                    && ghost.1.is_none())
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
        .args(["node", "assign", "w-1", "evaluator", &run_id.to_string()])
        .assert()
        .success();
    harness
        .cli()
        .args(["node", "assign", "w-2", "evaluator", &run_id.to_string()])
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
        .args(["run", "pause", &run_id.to_string()])
        .assert()
        .success();

    harness
        .wait_for(
            "paused run reconciles all nodes down",
            Duration::from_secs(10),
            || async {
                let w1 = harness.node_state("w-1").await?;
                let w2 = harness.node_state("w-2").await?;
                let ghost = harness.node_state("ghost-node").await?;
                Ok(w1.0.is_none()
                    && w1.1.is_none()
                    && w1.2.is_none()
                    && w1.3.is_none()
                    && w2.0.is_none()
                    && w2.1.is_none()
                    && w2.2.is_none()
                    && w2.3.is_none()
                    && ghost.0.is_none()
                    && ghost.1.is_none())
            },
        )
        .await?;

    harness
        .cli()
        .args(["node", "assign", "w-1", "evaluator", &run_id.to_string()])
        .assert()
        .success();
    harness
        .cli()
        .args(["node", "assign", "w-2", "evaluator", &run_id.to_string()])
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
        .args(["run", "pause", &run_id.to_string()])
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
        .args(["run", "remove", &run_id.to_string()])
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
