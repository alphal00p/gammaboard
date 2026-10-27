use super::*;

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn local_launch_survives_removing_the_running_executable_path() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    let original = harness.bin_path.clone();
    let executable = harness.artifacts.join(".replaceable-gammaboard");
    if std::fs::hard_link(&original, &executable).is_err() {
        std::fs::copy(&original, &executable)?;
    }
    harness.bin_path = executable.clone();
    let server = harness.start_server().await?;
    harness.bin_path = original;
    let cookie = login_cookie(&server).await?;
    std::fs::remove_file(&executable)?;
    let response = http_post_json(
        &server,
        "/api/nodes/auto-run",
        json!({"count":1,"name_prefix":"replaced"}),
        Some(&cookie),
    )
    .await?;
    let status = response.status();
    let body = response.text().await?;
    anyhow::ensure!(status.is_success(), "launch after unlink failed: {body}");
    let response: JsonValue = serde_json::from_str(&body)?;
    let node = response["node_names"][0].as_str().unwrap().to_owned();
    harness
        .wait_for(
            "worker registered after executable replacement",
            Duration::from_secs(15),
            || async {
                Ok(sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM nodes WHERE name=$1 AND lease_expires_at>now())",
                )
                .bind(&node)
                .fetch_one(&harness.pool)
                .await?)
            },
        )
        .await?;
    http_post_json(
        &server,
        &format!("/api/nodes/{node}/stop"),
        json!({}),
        Some(&cookie),
    )
    .await?
    .error_for_status()?;
    harness
        .wait_for(
            "replacement worker stopped",
            Duration::from_secs(15),
            || async {
                Ok(!sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM nodes WHERE name=$1 AND lease_expires_at>now())",
                )
                .bind(&node)
                .fetch_one(&harness.pool)
                .await?)
            },
        )
        .await?;
    harness.cleanup().await
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_server_queue_tuning_update_applies_to_active_sample_task() -> anyhow::Result<()>
{
    let mut harness = FullStackHarness::new().await?;

    let config = temp_config(
        r#"
name = "queue-tuning-live-update-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0
timing = { per_sample_seconds = 0.005 }

[[task_queue]]
name = "sample-a"
kind = "sample"
stop_condition = { max_samples = 8192 }
accumulator = { config = "scalar" }
sampler = { config = { kind = "naive_monte_carlo" } }

[sampler_aggregator_runner_params]
performance_snapshot_interval_ms = 100
min_tick_time_ms = 10
frontend_sync_interval_ms = 100

[sampler_aggregator_runner_params.queue]

target_batch_eval_ms = 50.0
max_batch_size = 32

max_batches_per_tick = 8
max_insert_bundle_size = 8
max_concurrent_insert_tasks = 2
completed_batch_fetch_limit = 64
"#,
    );

    harness.add_run(&config);
    let run_id = harness.run_id("queue-tuning-live-update-e2e").await?;
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
                    SELECT metrics
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
                    Ok(diag["queue_config"]["target_batch_eval_ms"].as_f64() == Some(50.0))
                }
            },
        )
        .await?;

    let update = http_post_json(
        &server_url,
        &format!("/api/runs/{run_id}/tasks/{task_id}/queue-tuning"),
        json!({
            "queue_tuning": {

                "target_batch_eval_ms": 500.0,
                "fixed_batch_size": 24,
                "max_batch_size": 256,
                "max_generation_size": 256
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
                    Ok(
                        task["queue_tuning"]["fixed_batch_size"].as_u64() == Some(24)
                            && task["queue_tuning"]["target_batch_eval_ms"].as_f64() == Some(500.0)
                            && task["queue_tuning"]["max_generation_size"].as_u64() == Some(256),
                    )
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
                        SELECT metrics
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
                    Ok(
                        diag["queue_config"]["target_batch_eval_ms"].as_f64() == Some(500.0)
                            && diag["queue_config"]["fixed_batch_size"].as_u64() == Some(24)
                            && diag["queue_config"]["max_generation_size"].as_u64() == Some(256),
                    )
                }
            },
        )
        .await?;

    harness.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_removes_child_runs_with_parent() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness
        .start_nodes(&["delete-family-s", "delete-family-e"])
        .await?;

    let config = temp_config(
        r#"
kind = "parameter_scan"
name = "delete-parent-run-e2e"
max_concurrent_runs = 1

[[parameters]]
name = "scale"
values = [
    1,
]

[measurement]
source_task = "sample"

[child]
run = '''
name = "delete-child-run-$(scale:1)"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[evaluator.timing]
per_sample_seconds = 0.02

[[task_queue]]
name = "sample"
kind = "sample"

[task_queue.stop_condition]
max_samples = 1000000

[task_queue.measurement]
selection = { kind = 'all_component_means' }

[task_queue.accumulator]
config = "scalar"

[task_queue.sampler.config]
kind = "naive_monte_carlo"
'''
"#,
    );

    harness.add_run(&config);
    let parent_run_id = harness.run_id("delete-parent-run-e2e").await?;

    harness
        .cli()
        .args(["node", "auto-assign", &parent_run_id.to_string()])
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
                let active: i64 = sqlx::query_scalar("SELECT count(*) FROM nodes n JOIN runs r ON r.id=n.active_run_id WHERE r.parent_run_id=$1")
                    .bind(parent_run_id).fetch_one(&harness.pool).await?;
                Ok(child_count == 1 && active == 2)
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
        .args(["run", "remove", "--yes", &parent_run_id.to_string()])
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

    harness.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_server_run_removal_handles_long_deletes_and_rolls_back_failures()
-> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    let result: anyhow::Result<()> = async {
        for name in ["large-parent", "large-child", "large-grandchild", "keep-run"] {
            harness.add_run(&temp_config(&format!("name = \"{name}\"\n")));
        }
        let parent = harness.run_id("large-parent").await?;
        let child = harness.run_id("large-child").await?;
        let grandchild = harness.run_id("large-grandchild").await?;
        let keep = harness.run_id("keep-run").await?;
        sqlx::query("UPDATE runs SET parent_run_id=$1 WHERE id=$2").bind(parent).bind(child).execute(&harness.pool).await?;
        sqlx::query("UPDATE runs SET parent_run_id=$1 WHERE id=$2").bind(child).bind(grandchild).execute(&harness.pool).await?;
        for (run, count) in [(child, 2048), (grandchild, 2048), (keep, 1)] {
            let task: i64 = sqlx::query_scalar("INSERT INTO run_tasks (run_id,name,sequence_nr,task,task_toml,state) VALUES ($1,'training',1,'{\"kind\":\"sample\",\"publish_result\":false,\"stop_condition\":{\"max_samples\":4096}}','', 'failed') RETURNING id")
                .bind(run).fetch_one(&harness.pool).await?;
            sqlx::query("INSERT INTO batches (run_id,task_id,batch_size,status) SELECT $1,$2,1,'completed' FROM generate_series(1,$3)")
                .bind(run).bind(task).bind(count).execute(&harness.pool).await?;
            sqlx::query("INSERT INTO batch_inputs (batch_id,latent_batch) SELECT id,decode(repeat('ab',1024),'hex') FROM batches WHERE run_id=$1")
                .bind(run).execute(&harness.pool).await?;
            sqlx::query("INSERT INTO batch_results (batch_id,batch_observable,completed_at) SELECT id,jsonb_build_object('padding',repeat('x',4096)),now() FROM batches WHERE run_id=$1")
                .bind(run).execute(&harness.pool).await?;
            sqlx::query("INSERT INTO run_sampler_checkpoints (run_id,task_id,sampler_checkpoint) VALUES ($1,$2,'{}')")
                .bind(run).bind(task).execute(&harness.pool).await?;
        }
        sqlx::raw_sql(&format!(r#"
            CREATE TABLE removal_fault (fail boolean NOT NULL);
            INSERT INTO removal_fault VALUES (true);
            CREATE FUNCTION exercise_run_removal() RETURNS trigger LANGUAGE plpgsql AS $$
            BEGIN
                IF OLD.id = {parent} THEN
                    IF (SELECT fail FROM removal_fault) THEN
                        RAISE EXCEPTION 'injected run removal failure';
                    END IF;
                    -- Exceed both normal 30-second deadlines while HTTP polling
                    -- must stay responsive and the controller lock must survive.
                    PERFORM pg_sleep(31);
                END IF;
                RETURN OLD;
            END;
            $$;
            CREATE TRIGGER exercise_run_removal BEFORE DELETE ON runs
            FOR EACH ROW EXECUTE FUNCTION exercise_run_removal();
        "#)).execute(&harness.pool).await?;
        let server_url = harness.start_server().await?;
        let cookie = login_cookie(&server_url).await?;
        let client = reqwest::Client::builder().timeout(Duration::from_secs(3)).build()?;
        let delete_url = format!("{server_url}/api/runs/{parent}");
        let response = client.delete(&delete_url).header("Cookie", &cookie).send().await?;
        anyhow::ensure!(response.status() == reqwest::StatusCode::ACCEPTED);
        let failed: JsonValue = serde_json::from_str(&response.text().await?)?;
        let failed_url = format!("{server_url}/api/run-removals/{}", failed["operation_id"].as_str().unwrap());
        harness.wait_for("failed removal reported to dashboard", Duration::from_secs(10), || async {
            let status: JsonValue = serde_json::from_str(&client.get(&failed_url).header("Cookie", &cookie).send().await?.error_for_status()?.text().await?)?;
            if status["status"] == "running" { return Ok(false); }
            anyhow::ensure!(status["status"] == "failed" && status["error"].as_str().unwrap().contains("injected run removal failure"), "unexpected result: {status}");
            Ok(true)
        }).await?;
        let counts: (i64,i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM runs),(SELECT count(*) FROM batches),(SELECT count(*) FROM batch_inputs),(SELECT count(*) FROM batch_results),(SELECT count(*) FROM run_sampler_checkpoints)")
            .fetch_one(&harness.pool).await?;
        anyhow::ensure!(counts == (4,4097,4097,4097,3), "failed deletion lost data: {counts:?}");
        sqlx::query("UPDATE removal_fault SET fail=false").execute(&harness.pool).await?;
        let response = client.delete(&delete_url).header("Cookie", &cookie).send().await?;
        anyhow::ensure!(response.status() == reqwest::StatusCode::ACCEPTED);
        let operation: JsonValue = serde_json::from_str(&response.text().await?)?;
        anyhow::ensure!(operation["operation_id"] != failed["operation_id"]);
        let status_url = format!("{server_url}/api/run-removals/{}", operation["operation_id"].as_str().unwrap());
        harness.wait_for("slow database deletion started", Duration::from_secs(10), || async {
            Ok(sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event='PgSleep')")
                .fetch_one(&harness.pool).await?)
        }).await?;
        let lock_owned: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity a JOIN pg_locks l ON l.pid=a.pid WHERE a.datname=current_database() AND a.wait_event='PgSleep' AND l.locktype='advisory' AND l.classid=1196245588 AND l.objid=1 AND l.granted)")
            .fetch_one(&harness.pool).await?;
        anyhow::ensure!(lock_owned, "deletion must use the transaction that owns the controller lock");
        let duplicate: JsonValue = serde_json::from_str(&client.delete(&delete_url).header("Cookie", &cookie).send().await?.error_for_status()?.text().await?)?;
        anyhow::ensure!(duplicate["operation_id"] == operation["operation_id"]);
        harness.wait_for("long dashboard deletion completed", Duration::from_secs(45), || async {
            let status: JsonValue = serde_json::from_str(&client.get(&status_url).header("Cookie", &cookie).send().await?.error_for_status()?.text().await?)?;
            anyhow::ensure!(status["status"] != "failed", "deletion failed: {status}");
            Ok(status["status"] == "completed")
        }).await?;
        let remaining: Vec<i32> = sqlx::query_scalar("SELECT id FROM runs ORDER BY id").fetch_all(&harness.pool).await?;
        anyhow::ensure!(remaining == vec![keep], "unrelated run was removed: {remaining:?}");
        let counts: (i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM batches),(SELECT count(*) FROM batch_inputs),(SELECT count(*) FROM batch_results),(SELECT count(*) FROM run_sampler_checkpoints)")
            .fetch_one(&harness.pool).await?;
        anyhow::ensure!(counts == (1,1,1,1), "unexpected remaining payloads: {counts:?}");
        Ok(())
    }.await;
    harness.cleanup().await?;
    result
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_server_run_removal_outlives_http_requests() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    for name in ["delete-parent", "delete-child", "keep-run"] {
        harness.add_run(&temp_config(&format!("name = \"{name}\"\n")));
    }
    let parent_id = harness.run_id("delete-parent").await?;
    let child_id = harness.run_id("delete-child").await?;
    let keep_id = harness.run_id("keep-run").await?;
    sqlx::query("UPDATE runs SET parent_run_id = $1 WHERE id = $2")
        .bind(parent_id)
        .bind(child_id)
        .execute(&harness.pool)
        .await?;
    sqlx::query(
        "INSERT INTO nodes (name, uuid, lease_expires_at) VALUES ('history-worker', 'history-worker', now() - interval '1 second')",
    )
    .execute(&harness.pool)
    .await?;
    sqlx::query(
        "INSERT INTO run_telemetry_workers (run_id, worker_id) VALUES ($1, 'history-worker')",
    )
    .bind(child_id)
    .execute(&harness.pool)
    .await?;
    sqlx::query(
        "INSERT INTO evaluator_performance_history (run_id, worker_id) VALUES ($1, 'history-worker')",
    )
    .bind(child_id)
    .execute(&harness.pool)
    .await?;

    let server_url = harness.start_server().await?;
    let cookie = login_cookie(&server_url).await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()?;
    let delete_url = format!("{server_url}/api/runs/{parent_id}");
    assert_eq!(
        client.delete(&delete_url).send().await?.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );

    // Hold the history cascade indefinitely: submission and status requests must
    // still finish before the HTTP client's short timeout.
    let mut blocker = harness.pool.begin().await?;
    sqlx::query("LOCK TABLE evaluator_performance_history IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await?;
    let response = client
        .delete(&delete_url)
        .header("Cookie", &cookie)
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
    let operation: JsonValue = serde_json::from_str(&response.text().await?)?;
    assert_eq!(operation["status"], "running");
    let operation_id = operation["operation_id"].as_str().unwrap();
    let status_url = format!("{server_url}/api/run-removals/{operation_id}");

    let duplicate = client
        .delete(&delete_url)
        .header("Cookie", &cookie)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let duplicate: JsonValue = serde_json::from_str(&duplicate)?;
    assert_eq!(duplicate["operation_id"], operation["operation_id"]);
    assert_eq!(
        client.get(&status_url).send().await?.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let pending = client
        .get(&status_url)
        .header("Cookie", &cookie)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let pending: JsonValue = serde_json::from_str(&pending)?;
    assert_eq!(pending["status"], "running");
    assert_eq!(harness.run_id("delete-parent").await?, parent_id);
    blocker.rollback().await?;

    harness
        .wait_for("background deletion", Duration::from_secs(10), || async {
            let status = client
                .get(&status_url)
                .header("Cookie", &cookie)
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            let status: JsonValue = serde_json::from_str(&status)?;
            anyhow::ensure!(status["status"] != "failed", "deletion failed: {status}");
            Ok(status["status"] == "completed")
        })
        .await?;
    let remaining: Vec<i32> = sqlx::query_scalar("SELECT id FROM runs ORDER BY id")
        .fetch_all(&harness.pool)
        .await?;
    assert_eq!(remaining, vec![keep_id]);
    let history_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM evaluator_performance_history")
            .fetch_one(&harness.pool)
            .await?;
    assert_eq!(history_count, 0);

    // Operations are local to the server; after a restart, instruct the client
    // to refresh rather than falsely reporting success or waiting forever.
    harness
        .kill_child(&format!(
            "server:{}",
            server_url.trim_start_matches("http://")
        ))
        .await?;
    let restarted_url = harness.start_server().await?;
    let restarted_cookie = login_cookie(&restarted_url).await?;
    let response = client
        .get(format!("{restarted_url}/api/run-removals/{operation_id}"))
        .header("Cookie", restarted_cookie)
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    assert!(response.text().await?.contains("Refresh the run list"));
    harness.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_server_auth_protects_pause_endpoint() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config = temp_config("name = \"auth-e2e\"\n");
    harness.add_run(&config);
    let run_id = harness.run_id("auth-e2e").await?;

    harness.start_node("w-1").await?;
    harness.assign_node("w-1", "evaluator", "auth-e2e");

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

    let unauthenticated_read = reqwest::get(format!("{server_url}/api/runs")).await?;
    assert_eq!(
        unauthenticated_read.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );

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

    let authenticated_read = http_get_with_cookie(&server_url, "/api/runs", &cookie).await?;
    assert!(authenticated_read.contains("\"run_name\":\"auth-e2e\""));
    assert!(authenticated_read.contains("\"queue_tuning_defaults\":"));
    for omitted_field in [
        "run_toml",
        "provenance",
        "integration_params",
        "domain",
        "target",
    ] {
        assert!(!authenticated_read.contains(&format!("\"{omitted_field}\":")));
    }

    // Server state is reconstructed from PostgreSQL; existing signed sessions
    // and independently running nodes remain usable after the server is killed.
    harness
        .kill_child(&format!(
            "server:{}",
            server_url.trim_start_matches("http://")
        ))
        .await?;
    let server_url = harness
        .start_server_with_auth((&password_hash, "test-session-secret"))
        .await?;
    let nodes = http_get_with_cookie(&server_url, "/api/nodes", &cookie).await?;
    assert!(nodes.contains("\"node_name\":\"w-1\""));
    assert!(
        http_get_with_cookie(&server_url, "/api/runs", &cookie)
            .await?
            .contains("auth-e2e")
    );

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

    harness.cleanup().await?;
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

    let node_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM nodes WHERE lease_expires_at > now()")
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
        json!({"launcher_id":"e2e-launcher"}),
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

    let token = claim_body["request"]["lease_token"].as_str().unwrap();
    for (name, job_id) in [("queued-w-1", "101"), ("queued-w-2", "102")] {
        for state in ["submitting", "submitted"] {
            let response = http_post_json(
                &server_url,
                &format!("/api/node-launch-requests/{request_id}/progress"),
                json!({"lease_token":token,"node_name":name,"state":state,
                    "job_id":(state=="submitted").then_some(job_id)}),
                Some(&cookie),
            )
            .await?;
            assert_eq!(response.status(), reqwest::StatusCode::OK);
        }
    }

    harness.start_nodes(&["queued-w-1", "queued-w-2"]).await?;
    harness
        .wait_for(
            "launch request is fulfilled after workers connect",
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
                Ok(state == Some("fulfilled"))
            },
        )
        .await?;

    harness.cleanup().await?;
    Ok(())
}
