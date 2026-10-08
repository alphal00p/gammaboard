use super::*;

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_alternating_havana_e2e() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    // Initial run with tasks 1..4:
    // 1: havana_training
    // 2: havana_inference
    // 3: naive_monte_carlo
    // 4: image
    let config = temp_config(
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

    harness.add_run(&config);
    let run_id = harness.run_id("havana-alt-e2e").await?;

    // Start nodes and assign roles
    harness.start_nodes(&["w-1", "w-2"]).await?;

    harness.assign_node("w-1", "sampler_aggregator", "havana-alt-e2e");
    harness.assign_node("w-2", "evaluator", "havana-alt-e2e");

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

    let task_file = temp_config(&tasks_toml);

    harness
        .cli()
        .args([
            "run",
            "task",
            "append",
            &run_id.to_string(),
            task_file.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // Reassign nodes so the newly appended tasks will be picked up by workers.
    // Use auto-assign to let the system pick appropriate nodes.
    harness
        .cli()
        .args(["node", "auto-assign", &run_id.to_string()])
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

    harness.cleanup().await?;
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

    harness.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_task_level_evaluator_switch_e2e() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness.start_nodes(&["w-1", "w-2"]).await?;

    let config = temp_config(
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

    harness.add_run(&config);
    let run_id = harness.run_id("task-level-evaluator-switch-e2e").await?;

    harness.assign_node(
        "w-1",
        "sampler_aggregator",
        "task-level-evaluator-switch-e2e",
    );
    harness.assign_node("w-2", "evaluator", "task-level-evaluator-switch-e2e");

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
            .get("components")
            .expect("components")
            .get(0)
            .expect("index 0")
            .get("state")
            .unwrap()
            .get("sum_weighted_value")
            .and_then(JsonValue::as_f64)
            .expect("sum_weighted_value");
        let count = state
            .get("components")
            .expect("components")
            .get(0)
            .expect("index 0")
            .get("state")
            .unwrap()
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

    harness.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_installation_smoke_produces_unit_estimate() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    let config = temp_config(include_str!(
        "../../resources/templates/runs/installation-smoke.toml"
    ));
    harness.add_run(&config);

    let run_id = harness.run_id("installation-smoke").await?;
    harness.start_nodes(&["w-1", "w-2"]).await?;
    harness.assign_node("w-1", "sampler_aggregator", "installation-smoke");
    harness.assign_node("w-2", "evaluator", "installation-smoke");

    harness
        .wait_for(
            "installation smoke task completes",
            Duration::from_secs(60),
            || {
                let pool = harness.pool.clone();
                async move {
                    let (state, failure_reason): (String, Option<String>) = sqlx::query_as(
                        "SELECT state, failure_reason FROM run_tasks WHERE run_id = $1 AND name = 'integrate-one'",
                    )
                    .bind(run_id)
                    .fetch_one(&pool)
                    .await?;
                    anyhow::ensure!(
                        state != "failed",
                        "installation smoke failed: {}",
                        failure_reason.unwrap_or_else(|| "no failure_reason".to_string())
                    );
                    Ok(state == "completed")
                }
            },
        )
        .await?;

    let completed_samples: i64 =
        sqlx::query_scalar("SELECT nr_completed_samples FROM runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(&harness.pool)
            .await?;
    assert_eq!(completed_samples, 10_000);

    let observable = harness
        .run_current_accumulator(run_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("installation smoke has no observable"))?;
    let state = observable
        .pointer("/components/0/state")
        .ok_or_else(|| anyhow::anyhow!("missing scalar accumulator state: {observable}"))?;
    let sum = state
        .get("sum_weighted_value")
        .and_then(JsonValue::as_f64)
        .ok_or_else(|| anyhow::anyhow!("missing scalar sum: {observable}"))?;
    let count = state
        .get("count")
        .and_then(JsonValue::as_i64)
        .ok_or_else(|| anyhow::anyhow!("missing scalar count: {observable}"))?;
    let estimate = sum / count as f64;
    assert!(
        (estimate - 1.0).abs() < 1e-12,
        "expected unit estimate, got {estimate}; observable={observable}"
    );

    harness.cleanup().await
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_reclaims_claimed_batches_after_worker_death() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config = temp_config(
        r#"
name = "worker-death-e2e"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0
timing = { per_sample_seconds = 0.1 }

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = 128 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }

[evaluator_runner_params]
performance_snapshot_interval_ms = 200

[sampler_aggregator_runner_params]
performance_snapshot_interval_ms = 200
min_tick_time_ms = 10
frontend_sync_interval_ms = 1000
[sampler_aggregator_runner_params.queue]
target_batch_eval_ms = 250.0
max_batch_size = 16
max_batches_per_tick = 4
completed_batch_fetch_limit = 64
"#,
    );

    harness.add_run(&config);
    let run_id = harness.run_id("worker-death-e2e").await?;

    harness.start_nodes(&["w-1", "w-2", "w-3"]).await?;

    harness.assign_node("w-1", "sampler_aggregator", "worker-death-e2e");
    harness.assign_node("w-2", "evaluator", "worker-death-e2e");

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

    harness.assign_node("w-3", "evaluator", "worker-death-e2e");

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

    harness.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_synthetic_training_windows_and_inference() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    for node in ["synthetic-s", "synthetic-e1", "synthetic-e2"] {
        harness.start_node(node).await?;
    }
    for (training, generation_size) in [(true, 32), (false, 32), (true, 512), (false, 512)] {
        let name = format!(
            "synthetic-{}-{generation_size}",
            if training { "training" } else { "inference" }
        );
        let config = temp_config(&format!(
            r#"
name = "{name}"
[evaluator]
kind = "unit"
timing = {{ per_sample_seconds = 0.00001, overhead_seconds = 0.001, sigma_overhead_seconds = 0.0001, seed = 42 }}
[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = {{ max_samples = 1024 }}
accumulator = {{ config = "scalar" }}
sampler_aggregator = {{ config = {{ kind = "naive_monte_carlo", seed = 42, training_window_samples = {window}, generation_timing = {{ overhead_seconds = 0.0001 }}, update_timing = {{ overhead_seconds = 0.002 }} }} }}
[evaluator_runner_params]
performance_snapshot_interval_ms = 20
[sampler_aggregator_runner_params]
performance_snapshot_interval_ms = 20
frontend_sync_interval_ms = 20
min_tick_time_ms = 1
[sampler_aggregator_runner_params.queue]
max_generation_size = {generation_size}
max_batch_size = 128
fixed_batch_size = 16

max_batches_per_tick = 2

target_batch_eval_ms = 1.0
"#,
            window = if training { 128 } else { 0 }
        ));
        harness.add_run(&config);
        let run_id = harness.run_id(&name).await?;
        harness.assign_node("synthetic-s", "sampler_aggregator", &name);
        harness.assign_node("synthetic-e1", "evaluator", &name);
        harness.assign_node("synthetic-e2", "evaluator", &name);
        harness.wait_for("synthetic task completes",Duration::from_secs(30),|| async {
            let completed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM run_tasks WHERE run_id=$1 AND state='completed')")
                .bind(run_id).fetch_one(&harness.pool).await?;
            Ok(completed)
        }).await?;
        let progress = harness.run_sample_progress(run_id).await?;
        assert_eq!(progress, (1024, 1024));
        let diagnostics: JsonValue = sqlx::query_scalar("SELECT engine_diagnostics FROM sampler_aggregator_performance_latest WHERE run_id=$1 ORDER BY created_at DESC LIMIT 1")
            .bind(run_id).fetch_one(&harness.pool).await?;
        assert_eq!(diagnostics["synthetic"], true);
        assert_eq!(
            diagnostics["training_updates"],
            if training { 8 } else { 0 }
        );
        assert_eq!(diagnostics["pending_training_samples"], 0);
        let expected_draws = 1024
            / if training {
                generation_size.min(128)
            } else {
                generation_size
            };
        assert_eq!(diagnostics["generation_timing"]["calls"], expected_draws);
        assert_eq!(
            diagnostics["ingest_timing"]["calls"],
            if training { expected_draws } else { 0 }
        );
        harness.wait_for("synthetic evaluator diagnostics flush", Duration::from_secs(10), || async {
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM evaluator_performance_latest WHERE run_id=$1 AND metrics->'engine_diagnostics'->>'synthetic'='true'")
                .bind(run_id).fetch_one(&harness.pool).await?;
            Ok(count>0)
        }).await?;
    }
    harness.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_training_retry_history_cannot_enter_sampling_task() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    // Keep training rows just as reclaiming a claim does in production. This
    // deliberately leaves more retained training samples than the next task's
    // entire budget, reproducing the GL30 progress-constraint failure.
    sqlx::raw_sql(
        "CREATE FUNCTION retain_training_retry_history() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN IF NEW.requires_training_values THEN NEW.retry_count := 1; END IF; RETURN NEW; END $$;
         CREATE TRIGGER retain_training_retry_history BEFORE INSERT ON batches
         FOR EACH ROW EXECUTE FUNCTION retain_training_retry_history();",
    ).execute(&harness.pool).await?;
    let name = "training-retry-history";
    let config = temp_config(
        r#"
name = "training-retry-history"
[evaluator]
kind = "unit"
[[task_queue]]
name = "training"
kind = "sample"
stop_condition = { max_samples = 128 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo", seed = 42, training_window_samples = 32 } }
[[task_queue]]
name = "sampling"
kind = "sample"
stop_condition = { max_samples = 64 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo", seed = 43 } }
[sampler_aggregator_runner_params]
frontend_sync_interval_ms = 20
min_tick_time_ms = 1
[sampler_aggregator_runner_params.queue]
max_batch_size = 16
"#,
    );
    harness.add_run(&config);
    let run_id = harness.run_id(name).await?;
    harness.start_nodes(&["retry-s", "retry-e"]).await?;
    harness.assign_node("retry-s", "sampler_aggregator", name);
    harness.assign_node("retry-e", "evaluator", name);
    harness
        .wait_for(
            "both tasks finish without replaying training results",
            Duration::from_secs(30),
            || async {
                let states: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT state,failure_reason FROM run_tasks WHERE run_id=$1 ORDER BY sequence_nr"
        ).bind(run_id).fetch_all(&harness.pool).await?;
                anyhow::ensure!(
                    !states.iter().any(|(state, _)| state == "failed"),
                    "task failure: {states:?}"
                );
                Ok(states.iter().all(|(state, _)| state == "completed"))
            },
        )
        .await?;
    let counts: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT nr_produced_samples,nr_completed_samples FROM run_tasks WHERE run_id=$1 ORDER BY sequence_nr"
    ).bind(run_id).fetch_all(&harness.pool).await?;
    assert_eq!(counts, vec![(128, 128), (64, 64)]);
    let retained: i64 = sqlx::query_scalar(
        "SELECT sum(batch_size)::bigint FROM batches WHERE run_id=$1 AND requires_training_values AND retry_count>0"
    ).bind(run_id).fetch_one(&harness.pool).await?;
    assert_eq!(retained, 128);
    let observable: JsonValue = sqlx::query_scalar(
        "SELECT s.observable_state FROM run_stage_snapshots s JOIN run_tasks t ON t.id=s.task_id
         WHERE t.run_id=$1 AND t.name='sampling' ORDER BY s.id DESC LIMIT 1",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(
        gammaboard::evaluation::AccumulatorState::from_json(&observable)?.sample_count(),
        64
    );
    harness.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_performance_cli_measures_ready_run_and_exports_history() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness.start_nodes(&["metrics-s", "metrics-e"]).await?;
    let config = temp_config(
        r#"
name = "metrics-inspection"
[evaluator]
kind = "unit"
continuous_dims = 2
timing = { overhead_seconds = 0.001 }
[evaluator_runner_params]
performance_snapshot_interval_ms = 100
[sampler_aggregator_runner_params]
min_tick_time_ms = 1
frontend_sync_interval_ms = 100
performance_snapshot_interval_ms = 100
[sampler_aggregator_runner_params.queue]
fixed_batch_size = 64
max_batch_size = 64
[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 100000000 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo", seed = 1234 } }
"#,
    );
    harness.add_run(&config);
    harness.assign_node("metrics-s", "sampler_aggregator", "metrics-inspection");
    harness.assign_node("metrics-e", "evaluator", "metrics-inspection");
    harness
        .cli()
        .args([
            "--json",
            "run",
            "wait",
            "metrics-inspection",
            "--evaluators",
            "1",
            "--timeout",
            "30s",
        ])
        .assert()
        .success();
    let measured = harness
        .cli()
        .args([
            "--json",
            "run",
            "performance",
            "metrics-inspection",
            "--duration",
            "2s",
            "--interval",
            "250ms",
        ])
        .assert()
        .success();
    let value: JsonValue = serde_json::from_slice(&measured.get_output().stdout)?;
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["valid"], true, "{value}");
    assert!(value["completed_samples"].as_i64().unwrap() > 0);
    assert!(value["samples_per_second"].as_f64().unwrap() > 0.0);
    assert_eq!(value["observed_evaluator_epoch_changes"], 0);
    assert!(
        value["evaluator_deltas"][0]["evaluate_seconds"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    let snapshot = value["snapshots"].as_array().unwrap().last().unwrap();
    for (collection, field) in [("evaluators", "metrics"), ("samplers", "runtime_metrics")] {
        let first = &value["snapshots"][0][collection][0][field]["busy"];
        let last = &snapshot[collection][0][field]["busy"];
        let seconds =
            last["elapsed_seconds"].as_f64().unwrap() - first["elapsed_seconds"].as_f64().unwrap();
        assert!(seconds > 0.0);
        for lane in ["compute_seconds", "io_seconds"] {
            let occupied = last[lane].as_f64().unwrap() - first[lane].as_f64().unwrap();
            assert!(
                occupied > 0.0 && occupied <= seconds,
                "{collection} {lane}: {occupied}/{seconds}"
            );
        }
    }
    let url = harness.start_server().await?;
    let cookie = login_cookie(&url).await?;
    let run_id = snapshot["run_id"].as_i64().unwrap();
    let panels: JsonValue = serde_json::from_str(
        &http_get_with_cookie(
            &url,
            &format!("/api/runs/{run_id}/performance?window_seconds=15"),
            &cookie,
        )
        .await?,
    )?;
    let updates = panels["updates"].as_array().unwrap();
    let activity = &updates
        .iter()
        .find(|v| v["panel"]["panel_id"] == "busy_rates")
        .unwrap()["panel"];
    assert_eq!(activity["rows"].as_array().unwrap().len(), 2);
    for row in activity["rows"].as_array().unwrap() {
        for index in [1, 2] {
            assert!((0.0..=100.0).contains(&row[index].as_f64().unwrap()));
        }
    }
    // More than one page of older records: the graph must not silently retain
    // only the latest five minutes or stop at the internal fetch-page boundary.
    sqlx::query(r#"
        INSERT INTO evaluator_performance_history(run_id, worker_id, created_at, metrics)
        SELECT $1, 'metrics-e', now() - interval '10000 seconds' + i * interval '1 second',
            jsonb_build_object('epoch','older-history','node_uuid','old-node','task_id','old-task',
                'samples_evaluated', i * 100,
                'busy',jsonb_build_object('elapsed_seconds',i,'compute_seconds',i * 0.5,'io_seconds',i * 0.1))
        FROM generate_series(0,5000) i
    "#).bind(run_id as i32).execute(&harness.pool).await?;
    let history_graphs: JsonValue = serde_json::from_str(
        &http_get_with_cookie(
            &url,
            &format!("/api/runs/{run_id}/performance/graphs"),
            &cookie,
        )
        .await?,
    )?;
    let graph = &history_graphs["states"][0];
    assert_eq!(graph["series"].as_array().unwrap().len(), 4);
    assert!(graph["series"][0]["points"].as_array().unwrap().len() <= 1200);
    let start = history_graphs["bounds"][0].as_i64().unwrap();
    let recorded_end = history_graphs["bounds"][1].as_i64().unwrap();
    assert!(recorded_end - start > 9_000_000);
    let points = graph["series"][0]["points"].as_array().unwrap();
    assert!(
        points
            .iter()
            .any(|p| p["x"].as_f64().unwrap() < start as f64 + 60_000.)
    );
    assert!(
        points
            .iter()
            .any(|p| (p["x"].as_f64().unwrap() - (start as f64 + 4_900_000.)).abs() < 60_000.)
    );
    let end = start + 4_900_500;
    let zoomed: JsonValue = serde_json::from_str(
        &http_get_with_cookie(
            &url,
            &format!(
                "/api/runs/{run_id}/performance/graphs?start_ms={}&end_ms={}",
                end - 50,
                end - 10
            ),
            &cookie,
        )
        .await?,
    )?;
    // A viewport narrower than the publication interval still uses its two
    // boundary snapshots, rather than disappearing or showing false zeroes.
    assert!(
        !zoomed["states"][0]["series"][0]["points"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        zoomed["bin_seconds"].as_f64().unwrap() < history_graphs["bin_seconds"].as_f64().unwrap()
    );
    let selected_summary: JsonValue = serde_json::from_str(
        &http_get_with_cookie(
            &url,
            &format!(
                "/api/runs/{run_id}/performance?start_ms={}&end_ms={}",
                end - 50,
                end - 10
            ),
            &cookie,
        )
        .await?,
    )?;
    assert_eq!(selected_summary["panels"][0]["panel_id"], "busy_rates");
    let selected_updates = selected_summary["updates"].as_array().unwrap();
    let busy = &selected_updates
        .iter()
        .find(|v| v["panel"]["panel_id"] == "busy_rates")
        .unwrap()["panel"];
    assert!((busy["rows"][0][1].as_f64().unwrap() - 50.).abs() < 1e-9);
    assert!((busy["rows"][0][2].as_f64().unwrap() - 10.).abs() < 1e-9);
    let window = &selected_updates
        .iter()
        .find(|v| v["panel"]["panel_id"] == "measurement_window")
        .unwrap()["panel"];
    let duration = window["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["key"] == "window_seconds")
        .unwrap();
    assert!((duration["value"].as_f64().unwrap() - 0.04).abs() < 1e-9);
    for seconds in [30., 300., 0.04] {
        let rolling: JsonValue = serde_json::from_str(
            &http_get_with_cookie(
                &url,
                &format!("/api/runs/{run_id}/performance/graphs?window_seconds={seconds}"),
                &cookie,
            )
            .await?,
        )?;
        assert_eq!(
            rolling["selection"][1].as_i64().unwrap() - rolling["selection"][0].as_i64().unwrap(),
            (seconds * 1000.) as i64
        );
    }
    assert_eq!(
        snapshot["samplers"][0]["runtime_metrics"]["batch_size_current"],
        64
    );
    let history = harness
        .cli()
        .args([
            "--json",
            "run",
            "performance",
            "metrics-inspection",
            "--since",
            "2020-01-01T00:00:00Z",
            "--limit",
            "1",
        ])
        .assert()
        .success();
    let history: JsonValue = serde_json::from_slice(&history.get_output().stdout)?;
    assert_eq!(history["truncated"], true);
    assert_eq!(history["rows"].as_array().unwrap().len(), 1);
    harness
        .cli()
        .args(["run", "pause", "metrics-inspection"])
        .assert()
        .success();
    harness
        .cli()
        .args([
            "--json",
            "run",
            "wait",
            "metrics-inspection",
            "--until",
            "idle",
        ])
        .assert()
        .success();
    let historical_summary: JsonValue = serde_json::from_str(
        &http_get_with_cookie(&url, &format!("/api/runs/{run_id}/performance"), &cookie).await?,
    )?;
    let busy = &historical_summary["updates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["panel"]["panel_id"] == "busy_rates")
        .unwrap()["panel"];
    assert!(
        busy["rows"][0][1].as_f64().is_some(),
        "paused runs retain selected measurements"
    );
    harness.cleanup().await?;
    Ok(())
}
