use super::*;

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn campaign_hard_cap_bounds_large_child_draws_and_covers_every_pilot() -> anyhow::Result<()> {
    for (max_active, window) in [(1, 64), (2, 64), (1, 4096)] {
        let mut harness = FullStackHarness::new().await?;
        let child = r#"
name = "budget-child"
[evaluator]
kind = "unit"
continuous_dims = 1
[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 4096 }
accumulator = { config = "scalar" }
sampler = { config = { kind = "naive_monte_carlo", seed = 17 } }
"#;
        let card = format!(
            r#"
kind = "integration_campaign"
name = "hard-cap"
stop_condition = {{ max_total_samples = 512 }}
allocation = {{ min_samples_per_child = 32, allocation_window_samples = {window}, max_active_runs = {max_active} }}
[[children]]
name = "positive"
coefficient = 1.0
run = '''{child}'''
[[children]]
name = "negative"
coefficient = -0.5
run = '''{child}'''
"#
        );
        harness.add_run(&temp_config(&card));
        let parent = harness.run_id("hard-cap").await?;
        harness
            .start_nodes(&["cap-parent", "cap-s1", "cap-s2", "cap-e1", "cap-e2"])
            .await?;
        harness
            .cli()
            .args(["node", "auto-assign", &parent.to_string()])
            .assert()
            .success();
        if max_active == 2 {
            harness
                .cli()
                .args([
                    "node",
                    "assign",
                    "cap-s2",
                    "sampler_aggregator",
                    &parent.to_string(),
                ])
                .assert()
                .success();
        }
        wait_for_task_state(&harness, parent, "completed", Duration::from_secs(90)).await?;
        let counts: Vec<(i64,i64)> = sqlx::query_as("SELECT t.nr_produced_samples,t.nr_completed_samples FROM run_tasks t JOIN runs r ON r.id=t.run_id WHERE r.parent_run_id=$1")
            .bind(parent).fetch_all(&harness.pool).await?;
        let result: JsonValue =
            sqlx::query_scalar("SELECT measurement_output FROM run_tasks WHERE run_id=$1")
                .bind(parent)
                .fetch_one(&harness.pool)
                .await?;
        harness.cleanup().await?;
        assert_eq!(counts.len(), 2);
        assert_eq!(
            counts.iter().map(|(produced, _)| produced).sum::<i64>(),
            512
        );
        assert_eq!(
            counts.iter().map(|(_, completed)| completed).sum::<i64>(),
            512
        );
        assert!(counts.iter().all(|(_, completed)| *completed >= 32));
        assert_eq!(result["results"][0]["value"], 0.5);
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Python"]
async fn campaign_waits_for_the_new_publishing_stage_to_initialize() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    let python = std::env::var("GAMMABOARD_PROCESS_PYTHON").unwrap_or_else(|_| "python3".into());
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/json_worker.py")
        .display()
        .to_string();
    let child = format!(
        r#"
name = "publication-child"
[evaluator]
kind = "unit"
continuous_dims = 1
[[task_queue]]
name = "training"
kind = "sample"
publish_result = false
stop_condition = {{ max_samples = 32 }}
accumulator = {{ config = "scalar" }}
sampler = {{ config = {{ kind = "naive_monte_carlo" }} }}
[[task_queue]]
name = "inference"
kind = "sample"
stop_condition = {{ max_samples = 32 }}
accumulator = {{ config = "scalar" }}
evaluator = {{ config = {{ kind = "unit", continuous_dims = 1, value_coordinate = 0 }} }}
sampler = {{ config = {{ kind = "process_sampler", command = [{python:?}, "-u", {fixture:?}, "sampler"], args = {{ initialize_delay_seconds = 2.0 }} }} }}
"#
    );
    let parent = format!(
        r#"
kind = "integration_campaign"
name = "publication-parent"
stop_condition = {{ max_total_samples = 1024, absolute_error = 1e-12 }}
allocation = {{ min_samples_per_child = 8, allocation_window_samples = 8 }}
[[children]]
name = "only"
run = '''{child}'''
"#
    );
    harness.add_run(&temp_config(&parent));
    let run = harness.run_id("publication-parent").await?;
    harness
        .start_nodes(&["publication-parent", "publication-s", "publication-e"])
        .await?;
    harness
        .cli()
        .args(["node", "auto-assign", &run.to_string()])
        .assert()
        .success();
    wait_for_task_state(&harness, run, "completed", Duration::from_secs(30)).await?;
    let result: JsonValue =
        sqlx::query_scalar("SELECT measurement_output FROM run_tasks WHERE run_id=$1")
            .bind(run)
            .fetch_one(&harness.pool)
            .await?;
    let samples: i64 = sqlx::query_scalar("SELECT COALESCE(sum(t.nr_completed_samples),0)::bigint FROM run_tasks t JOIN runs r ON r.id=t.run_id WHERE r.parent_run_id=$1 AND t.name='inference'")
        .bind(run).fetch_one(&harness.pool).await?;
    harness.cleanup().await?;
    assert!(
        samples > 0,
        "campaign converged before inference published anything"
    );
    assert_eq!(
        result["results"][0]["value"], 0.5,
        "training's constant-one result leaked into inference"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_integration_campaign_persists_a_provenanced_result() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness
        .start_nodes(&["campaign-parent", "campaign-s1", "campaign-e1"])
        .await?;

    let config = temp_config(
        r#"
kind = "integration_campaign"
name = "integration-campaign-result-e2e"
target = { kind = "scalar", value = 1.5 }

[measurement]

[stop_condition]
min_total_samples = 16
max_total_samples = 64
absolute_error = 1e-12

[allocation]
algorithm = "largest_variance"
max_active_runs = 1
allocation_window_samples = 8
min_samples_per_child = 8

[[children]]
name = "left"
coefficient = 2.0

run = '''
name = "campaign-result-left"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
name = "sample"
kind = "sample"

[task_queue.stop_condition]
max_samples = 32

[task_queue.measurement]
selection = { kind = 'all_component_means' }

[task_queue.accumulator]
config = "scalar"

[task_queue.sampler.config]
kind = "naive_monte_carlo"
seed = 1

'''
[[children]]
name = "right"
coefficient = -0.5

run = '''
name = "campaign-result-right"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
name = "sample"
kind = "sample"

[task_queue.stop_condition]
max_samples = 32

[task_queue.measurement]
selection = { kind = 'all_component_means' }

[task_queue.accumulator]
config = "scalar"

[task_queue.sampler.config]
kind = "naive_monte_carlo"
seed = 2
'''
"#,
    );

    harness.add_run(&config);
    let run_id = harness.run_id("integration-campaign-result-e2e").await?;
    harness
        .cli()
        .args(["node", "auto-assign", &run_id.to_string()])
        .assert()
        .success();

    wait_for_task_state(&harness, run_id, "completed", Duration::from_secs(90)).await?;

    let output: JsonValue = sqlx::query_scalar(
        "SELECT controller_output FROM run_tasks WHERE run_id = $1 AND task->>'kind' = 'integration_campaign'",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(
        output["combined_measurement"]["results"][0]["value"],
        json!(1.5)
    );
    assert!(output["result_snapshot_id"].is_string());
    assert!(
        output["children"]
            .as_array()
            .is_some_and(|children| children.iter().all(|child| {
                child["result_source"]["sample_count"]
                    .as_i64()
                    .is_some_and(|count| count > 0)
            }))
    );

    let result: JsonValue = sqlx::query_scalar(
        "SELECT persisted_observable FROM persisted_observable_snapshots WHERE id = $1",
    )
    .bind(
        output["result_snapshot_id"]
            .as_str()
            .unwrap()
            .parse::<i64>()?,
    )
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(result["sources"].as_array().map(Vec::len), Some(2));
    assert_eq!(result["metrics"][0]["value"], json!(1.5));

    let target: JsonValue = sqlx::query_scalar("SELECT target FROM runs WHERE id = $1")
        .bind(run_id)
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(target, json!({"kind": "scalar", "value": 1.5}));
    let child_targets: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM runs WHERE parent_run_id = $1 AND target IS NOT NULL",
    )
    .bind(run_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(
        child_targets, 0,
        "combined target must not be inherited by children"
    );

    let server_url = harness.start_server().await?;
    let cookie = login_cookie(&server_url).await?;
    let exported = http_get_with_cookie(
        &server_url,
        &format!("/api/runs/{run_id}/definition"),
        &cookie,
    )
    .await?;
    let exported: JsonValue = serde_json::from_str(&exported)?;
    let config =
        gammaboard::api::runs::parse_run_add_config_toml(exported["toml"].as_str().unwrap())?;
    assert_eq!(config.target, Some(target));

    let child_id = harness.run_id("campaign-result-left").await?;
    sqlx::query("UPDATE runs SET provenance = provenance || $2 WHERE id = $1")
        .bind(child_id)
        .bind(json!({"evaluator_metadata": {"graph_groups": null, "label": "retained"}}))
        .execute(&harness.pool)
        .await?;
    let exported = http_get_with_cookie(
        &server_url,
        &format!("/api/runs/{child_id}/definition"),
        &cookie,
    )
    .await?;
    let exported: JsonValue = serde_json::from_str(&exported)?;
    let raw = exported["toml"].as_str().unwrap();
    let child_config = gammaboard::api::runs::parse_run_add_config_toml(raw)?;
    assert_eq!(child_config.name, "campaign-result-left");
    assert_eq!(child_config.integration().unwrap().task_queue.len(), 1);
    let document: toml::Value = toml::from_str(raw)?;
    assert!(document.get("gammaboard").is_none());
    // Controller duplication starts a new orchestration; child duplication is standalone.
    assert_fresh_controller_duplicate(&harness, "integration-campaign-result-e2e").await?;
    harness
        .cli()
        .args([
            "run",
            "duplicate",
            "campaign-result-left",
            "standalone-copy",
        ])
        .assert()
        .success();
    let standalone_id = harness.run_id("standalone-copy").await?;
    let parent: Option<i32> = sqlx::query_scalar("SELECT parent_run_id FROM runs WHERE id=$1")
        .bind(standalone_id)
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(parent, None);
    let store = gammaboard::PgStore::new(harness.pool.clone());
    let tasks = gammaboard::api::runs::parse_task_queue_toml(
        "[task]\nkind='set_accumulator'\naccumulator='scalar'",
    )?;
    assert!(
        gammaboard::api::runs::append_tasks(&store, child_id, tasks)
            .await
            .is_err()
    );
    assert!(
        gammaboard::api::runs::update_task_queue_tuning(&store, child_id, 0, None)
            .await
            .is_err()
    );

    harness.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_campaign_recovers_from_sampler_and_evaluator_loss() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness.start_nodes(&["recovery-a", "recovery-b"]).await?;

    let config = temp_config(
        r#"
kind = "integration_campaign"
name = "campaign-worker-recovery-e2e"

[measurement]

[stop_condition]
max_total_samples = 1024

[allocation]
algorithm = "largest_variance"
max_active_runs = 1
min_evaluators_per_run = 1
allocation_window_samples = 64
min_samples_per_child = 64

[[children]]
name = "left"
coefficient = 2.0

run = '''
name = "campaign-recovery-left"

[evaluator]
kind = "unit"
continuous_dims = 1
timing = { per_sample_seconds = 0.01 }

[sampler_aggregator_runner_params]
frontend_sync_interval_ms = 20
performance_snapshot_interval_ms = 20

[sampler_aggregator_runner_params.queue]

max_batch_size = 16
target_batch_eval_ms = 50.0

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 512 }
measurement = { selection = { kind = 'all_component_means' } }
accumulator = { config = "scalar" }
sampler = { config = { kind = "naive_monte_carlo", seed = 1 } }

'''
[[children]]
name = "right"
coefficient = -0.5

run = '''
name = "campaign-recovery-right"

[evaluator]
kind = "unit"
continuous_dims = 1
timing = { per_sample_seconds = 0.01 }

[sampler_aggregator_runner_params]
frontend_sync_interval_ms = 20
performance_snapshot_interval_ms = 20

[sampler_aggregator_runner_params.queue]

max_batch_size = 16
target_batch_eval_ms = 50.0

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 512 }
measurement = { selection = { kind = 'all_component_means' } }
accumulator = { config = "scalar" }
sampler = { config = { kind = "naive_monte_carlo", seed = 2 } }
'''
"#,
    );

    harness.add_run(&config);
    let parent_id = harness.run_id("campaign-worker-recovery-e2e").await?;
    harness
        .cli()
        .args(["node", "auto-assign", &parent_id.to_string()])
        .assert()
        .success();

    harness
        .wait_for(
            "campaign has an in-flight evaluator batch before worker loss",
            Duration::from_secs(30),
            || async {
                let ready: bool = sqlx::query_scalar(
                    r#"
                    SELECT
                        (SELECT count(*) FROM nodes n
                         JOIN runs r ON r.id = n.active_run_id
                         WHERE r.parent_run_id = $1
                           AND n.lease_expires_at > now()
                           AND n.active_role IN ('sampler_aggregator', 'evaluator')) = 2
                        AND EXISTS(
                            SELECT 1 FROM batches b
                            JOIN runs r ON r.id = b.run_id
                            WHERE r.parent_run_id = $1 AND b.status = 'claimed'
                        )
                        AND (SELECT COALESCE(sum(r.nr_completed_samples), 0)
                             FROM runs r WHERE r.parent_run_id = $1) >= 64
                    "#,
                )
                .bind(parent_id)
                .fetch_one(&harness.pool)
                .await?;
                Ok(ready)
            },
        )
        .await?;

    let lost_nodes: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT n.name
        FROM nodes n
        JOIN runs r ON r.id = n.active_run_id
        WHERE r.parent_run_id = $1
          AND n.lease_expires_at > now()
          AND n.active_role IN ('sampler_aggregator', 'evaluator')
        ORDER BY n.name
        "#,
    )
    .bind(parent_id)
    .fetch_all(&harness.pool)
    .await?;
    anyhow::ensure!(lost_nodes.len() == 2, "expected two active child workers");

    for node in &lost_nodes {
        harness.kill_child(node).await?;
        sqlx::query(
            "UPDATE nodes SET lease_expires_at = now() - interval '1 second' WHERE name=$1",
        )
        .bind(node)
        .execute(&harness.pool)
        .await?;
    }
    for node in &lost_nodes {
        harness.start_node(node).await?;
    }

    wait_for_task_state(&harness, parent_id, "completed", Duration::from_secs(120)).await?;

    let failed_tasks: i64 = sqlx::query_scalar(
        r#"
        SELECT count(*)
        FROM run_tasks t
        JOIN runs r ON r.id = t.run_id
        WHERE (r.id = $1 OR r.parent_run_id = $1) AND t.state = 'failed'
        "#,
    )
    .bind(parent_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(failed_tasks, 0);

    let child_progress: (i64, i64, i64) = sqlx::query_as(
        r#"
        SELECT count(*), sum(nr_produced_samples)::bigint, sum(nr_completed_samples)::bigint
        FROM runs
        WHERE parent_run_id = $1
        "#,
    )
    .bind(parent_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(child_progress, (2, 1024, 1024));

    let open_batches: i64 = sqlx::query_scalar(
        r#"
        SELECT count(*)
        FROM batches b
        JOIN runs r ON r.id = b.run_id
        WHERE r.parent_run_id = $1 AND b.status IN ('pending', 'claimed')
        "#,
    )
    .bind(parent_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(open_batches, 0);

    let output: JsonValue = sqlx::query_scalar(
        "SELECT controller_output FROM run_tasks WHERE run_id=$1 AND task->>'kind'='integration_campaign'",
    )
    .bind(parent_id)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(
        output["combined_measurement"]["results"][0]["value"],
        json!(1.5)
    );
    assert_eq!(output["children"].as_array().map(Vec::len), Some(2));
    assert!(output["result_snapshot_id"].is_string());

    let inconsistent_progress: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM runtime_logs l
            JOIN runs r ON r.id = l.run_id
            WHERE (r.id = $1 OR r.parent_run_id = $1)
              AND l.fields::text LIKE '%run_tasks_progress_check%'
        )
        "#,
    )
    .bind(parent_id)
    .fetch_one(&harness.pool)
    .await?;
    assert!(!inconsistent_progress);

    harness.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn campaign_follows_child_stages_and_freezes_shared_file() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    let dir = tempfile::tempdir()?;
    let child_path = dir.path().join("child.toml");
    std::fs::write(
        &child_path,
        r#"name = "child-$(value:9)"

[replacements]
value = 9
final_name = "default-final"

[evaluator]
kind = "symbolica"
args = ["x"]

[evaluator.source]
kind = "expression"
expr = "1"

[[task_queue]]
name = "training"
kind = "sample"

[task_queue.stop_condition]
max_samples = 32

[task_queue.accumulator]
config = "scalar"

[task_queue.sampler.config]
kind = "naive_monte_carlo"
seed = 1

[[task_queue]]
name = "$(final_name:default-final)"
kind = "sample"

[task_queue.stop_condition]
max_samples = 16

[task_queue.evaluator.config]
kind = "symbolica"
args = ["x"]

[task_queue.evaluator.config.source]
kind = "expression"
expr = "0 + $(value:9)"

[task_queue.accumulator]
config = "scalar"

[task_queue.sampler.config]
kind = "naive_monte_carlo"
seed = 2
"#,
    )?;
    let document = format!(
        r#"kind = "integration_campaign"
name = "stage-campaign"

[stop_condition]
absolute_error = 1e-10
max_total_samples = 10000

[allocation]
min_evaluators_per_run = 1
min_samples_per_child = 4
allocation_window_samples = 8

[[children]]
name = "left"

[children.replacements]
value = 2
final_name = "left-inference"

[children.run]
file = "{}"

[[children]]
name = "right"
coefficient = -0.5

[children.replacements]
value = 3
final_name = "right-inference"

[children.run]
file = "{}"

[[children]]
name = "inline"
run = "name = \"inline-stages\"\n\n[evaluator]\nkind = \"symbolica\"\nargs = [\"x\"]\n\n[evaluator.source]\nkind = \"expression\"\nexpr = \"0\"\n\n[[task_queue]]\nname = \"excluded-training\"\nkind = \"sample\"\npublish_result = false\n\n[task_queue.stop_condition]\nmax_samples = 32\n\n[task_queue.accumulator]\nconfig = \"scalar\"\n\n[task_queue.sampler.config]\nkind = \"naive_monte_carlo\"\n\n[[task_queue]]\nname = \"inline-inference\"\nkind = \"sample\"\n\n[task_queue.stop_condition]\nmax_samples = 16\n\n[task_queue.accumulator]\nconfig = \"scalar\"\n\n[task_queue.sampler.config]\nkind = \"naive_monte_carlo\"\n"
"#,
        child_path.display(),
        child_path.display()
    );
    let config = temp_config(&document);
    harness.add_run(&config);
    // Children have not been spawned: restart/creation must use persisted contents.
    std::fs::remove_file(&child_path)?;
    harness.start_nodes(&["stage-s", "stage-e"]).await?;
    let parent_id = harness.run_id("stage-campaign").await?;
    harness
        .cli()
        .args(["node", "auto-assign", &parent_id.to_string()])
        .assert()
        .success();
    wait_for_task_state(&harness, parent_id, "completed", Duration::from_secs(60)).await?;
    let output: JsonValue =
        sqlx::query_scalar("SELECT controller_output FROM run_tasks WHERE run_id = $1")
            .bind(parent_id)
            .fetch_one(&harness.pool)
            .await?;
    assert_eq!(
        output["combined_measurement"]["results"][0]["value"],
        json!(0.5)
    );
    let total = output["total_samples"].as_i64().unwrap();
    // Window-bounded generation can converge before exhausting inference's
    // individual task caps. All training work and final-stage pilots must count.
    assert!((108..=144).contains(&total));
    for child in output["children"].as_array().unwrap() {
        let task_id: i64 = child["result_source"]["task_id"]
            .as_str()
            .unwrap()
            .parse()?;
        let name: String = sqlx::query_scalar("SELECT name FROM run_tasks WHERE id = $1")
            .bind(task_id)
            .fetch_one(&harness.pool)
            .await?;
        assert!(
            name.ends_with("-inference"),
            "unexpected published source: {name}"
        );
    }
    // Controller runs have no user-editable integration queue.
    let appended = temp_config("[[task_queue]]\nkind = 'set_accumulator'\naccumulator = 'scalar'");
    harness
        .cli()
        .args([
            "run",
            "task",
            "append",
            &parent_id.to_string(),
            appended.path().to_str().unwrap(),
        ])
        .assert()
        .failure();
    harness.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn child_source_forms_execute_identically_across_controllers() -> anyhow::Result<()> {
    use std::io::Write;
    let child = r#"
name = "source-child-$(scale:1)"
replacements = { scale = 1, samples = 4 }
[evaluator]
kind = "unit"
continuous_dims = "$(scale:1)"
discrete_dims = 0
[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = "$(samples:4)" }
measurement = { selection = { kind = 'all_component_means' } }
accumulator = { config = "scalar" }
sampler = { config = { kind = "naive_monte_carlo", seed = 0 } }
"#;
    for kind in [
        "integration_campaign",
        "parameter_scan",
        "hyperparameter_tuning",
    ] {
        let mut results = Vec::new();
        for from_file in [false, true] {
            let mut harness = FullStackHarness::new().await?;
            let dir = tempfile::tempdir()?;
            let child_path = dir.path().join("child.toml");
            std::fs::write(&child_path, child)?;
            let source = if from_file {
                "{ file = \"child.toml\" }".to_string()
            } else {
                format!("'''{child}'''")
            };
            let settings = match kind {
                "integration_campaign" => {
                    "stop_condition = { max_total_samples = 12 }\nallocation = { min_evaluators_per_run = 1, min_samples_per_child = 1 }\n[[children]]\nname = 'a'"
                }
                "parameter_scan" => "parameters = [{ name = 'scale', values = [3] }]\n[child]",
                "hyperparameter_tuning" => {
                    "optimizer = { algorithm = 'grid_search' }\nobjective = { source_task = 'sample', mode = 'minimize', selection = { kind = 'all_component_means' } }\nparameters = { scale = { kind = 'integer', min = 3, max = 3, step = 1 } }\n[child]"
                }
                _ => unreachable!(),
            };
            let document = format!(
                "kind = '{kind}'\nname = 'source-parent'\nreplacements = {{ scale = 99 }}\n{settings}\nreplacements = {{ scale = 2, samples = 12 }}\nrun = {source}\n"
            );
            let mut config = NamedTempFile::new_in(dir.path())?;
            config.write_all(document.as_bytes())?;
            harness.add_run(&config);
            let parent_id = harness.run_id("source-parent").await?;
            // Freeze on submission: neither input file exists when workers spawn children.
            std::fs::remove_file(child_path)?;
            drop(config);
            let task: JsonValue =
                sqlx::query_scalar("SELECT task FROM run_tasks WHERE run_id = $1")
                    .bind(parent_id)
                    .fetch_one(&harness.pool)
                    .await?;
            let frozen = if kind == "integration_campaign" {
                &task["children"][0]["run"]
            } else {
                &task["child"]["run"]
            };
            assert!(
                frozen.is_string(),
                "{kind}: persisted child source must be inline TOML"
            );
            harness.start_nodes(&["source-s", "source-e"]).await?;
            harness
                .cli()
                .args(["node", "auto-assign", &parent_id.to_string()])
                .assert()
                .success();
            wait_for_task_state(&harness, parent_id, "completed", Duration::from_secs(60)).await?;
            let children: Vec<JsonValue> = sqlx::query_scalar(
                r#"
                SELECT jsonb_build_object(
                    'name', r.name, 'integration_params', r.integration_params,
                    'point_spec', r.point_spec,
                    'tasks', (SELECT jsonb_agg(jsonb_build_object(
                        'name', t.name, 'task', t.task, 'state', t.state,
                        'samples', t.nr_completed_samples) ORDER BY t.sequence_nr)
                        FROM run_tasks t WHERE t.run_id = r.id))
                FROM runs r WHERE parent_run_id = $1 ORDER BY r.name
            "#,
            )
            .bind(parent_id)
            .fetch_all(&harness.pool)
            .await?;
            assert_eq!(children.len(), 1, "{kind}");
            assert_eq!(children[0]["tasks"][0]["samples"], json!(12), "{kind}");
            assert_eq!(
                children[0]["name"],
                json!(if kind == "integration_campaign" {
                    "source-child-2"
                } else {
                    "source-child-3"
                })
            );
            results.push(children);
            harness.cleanup().await?;
        }
        assert_eq!(
            results[0], results[1],
            "{kind}: inline/file execution differed"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_campaign_leader_handover_and_operator_controls() -> anyhow::Result<()> {
    use gammaboard::core::{ControlPlaneStore, WorkerRole};
    let mut harness = FullStackHarness::new().await?;
    let child = r#"
name = "lifecycle-child"
[evaluator]
kind = "unit"
continuous_dims = 1
timing = { overhead_seconds = 0.01 }
[[task_queue]]
name = "train"
kind = "sample"
stop_condition = { max_samples = 100000000 }
accumulator = { config = "scalar" }
sampler = { config = { kind = "naive_monte_carlo" } }
[task_queue.queue_tuning]
max_batch_size = 100
"#;
    let mut card = String::from(
        "kind = 'integration_campaign'\nname = 'lifecycle-parent'\nstop_condition = { max_total_samples = 1000000000 }\nallocation = { min_evaluators_per_run = 1, min_samples_per_child = 50000000, allocation_window_samples = 1000000 }\n",
    );
    for i in 0..11 {
        card.push_str(&format!(
            "[[children]]\nname = 'child-{i}'\nrun = '''{child}'''\n"
        ));
    }
    harness.add_run(&temp_config(&card));
    let parent = harness.run_id("lifecycle-parent").await?;
    // Deliberately keep the first leader inside child creation while a lower
    // name registers. This used to make both leaders create the same children.
    sqlx::raw_sql("CREATE FUNCTION slow_child_creation() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.parent_run_id IS NOT NULL THEN PERFORM pg_sleep(0.2); END IF; RETURN NEW; END $$; CREATE TRIGGER slow_child BEFORE INSERT ON runs FOR EACH ROW EXECUTE FUNCTION slow_child_creation();").execute(&harness.pool).await?;
    harness.start_nodes(&["z-sampler", "z-evaluator"]).await?;
    harness.assign_node("z-sampler", "sampler_aggregator", "lifecycle-parent");
    harness.assign_node("z-evaluator", "evaluator", "lifecycle-parent");
    harness
        .wait_for("first child", Duration::from_secs(15), || async {
            Ok(
                sqlx::query_scalar::<_, i64>("SELECT count(*) FROM runs WHERE parent_run_id=$1")
                    .bind(parent)
                    .fetch_one(&harness.pool)
                    .await?
                    > 0,
            )
        })
        .await?;
    harness.start_node("a-new-leader").await?;
    harness.wait_for("all eleven children and work", Duration::from_secs(20), || async {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE parent_run_id=$1").bind(parent).fetch_one(&harness.pool).await?;
        let samples: i64 = sqlx::query_scalar("SELECT coalesce(sum(nr_completed_samples),0)::bigint FROM runs WHERE parent_run_id=$1").bind(parent).fetch_one(&harness.pool).await?;
        Ok(count >= 11 && samples > 100)
    }).await?;
    let store = gammaboard::PgStore::new(harness.pool.clone());
    let selected = store
        .get_desired_assignment("z-sampler")
        .await?
        .unwrap()
        .run_id;
    sleep(Duration::from_secs(2)).await;
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT count(*),count(DISTINCT spawn_label) FROM runs WHERE parent_run_id=$1",
    )
    .bind(parent)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(counts, (11, 11));
    assert_eq!(
        store
            .get_desired_assignment("z-sampler")
            .await?
            .unwrap()
            .run_id,
        selected,
        "first publication must not end the million-sample window"
    );
    assert!(
        store
            .get_desired_assignment("a-new-leader")
            .await?
            .is_none(),
        "idle nodes must not be stolen"
    );
    store.clear_desired_assignment("z-evaluator").await?;
    harness
        .wait_for(
            "explicit unassign drains",
            Duration::from_secs(10),
            || async {
                Ok(sqlx::query_scalar::<_, bool>(
                    "SELECT active_run_id IS NULL FROM nodes WHERE name='z-evaluator'",
                )
                .fetch_one(&harness.pool)
                .await?)
            },
        )
        .await?;
    sleep(Duration::from_secs(1)).await;
    assert!(store.get_desired_assignment("z-evaluator").await?.is_none());
    store
        .assign_worker_pool("z-evaluator", WorkerRole::Evaluator, parent)
        .await?;
    harness
        .wait_for(
            "parent-assigned evaluator joins child",
            Duration::from_secs(10),
            || async {
                Ok(store
                    .get_desired_assignment("z-evaluator")
                    .await?
                    .is_some_and(|a| a.run_id == selected))
            },
        )
        .await?;
    store.clear_desired_assignment("z-sampler").await?;
    harness
        .wait_for(
            "sampler unassign parks remaining pool",
            Duration::from_secs(15),
            || async {
                Ok(store
                    .get_desired_assignment("z-evaluator")
                    .await?
                    .is_some_and(|a| a.run_id == parent)
                    && sqlx::query_scalar::<_, bool>(
                        "SELECT active_run_id IS NULL FROM nodes WHERE name='z-sampler'",
                    )
                    .fetch_one(&harness.pool)
                    .await?)
            },
        )
        .await?;
    store
        .assign_worker_pool("z-sampler", WorkerRole::SamplerAggregator, parent)
        .await?;
    harness
        .wait_for(
            "sampler reassignment restores retained pool",
            Duration::from_secs(15),
            || async {
                Ok(store
                    .get_desired_assignment("z-evaluator")
                    .await?
                    .is_some_and(|a| a.run_id == selected))
            },
        )
        .await?;
    gammaboard::api::runs::pause_run(&store, parent).await?;
    harness.wait_for("parent pause drains all descendants", Duration::from_secs(15), || async {
        Ok(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM nodes WHERE desired_run_id IS NOT NULL OR active_run_id IS NOT NULL").fetch_one(&harness.pool).await? == 0)
    }).await?;
    sleep(Duration::from_secs(1)).await;
    assert!(store.list_desired_assignments(None).await?.is_empty());
    use gammaboard::core::RunReadStore;
    assert_eq!(
        store
            .get_run_progress(parent)
            .await?
            .unwrap()
            .lifecycle_state,
        gammaboard::stores::RunLifecycleState::Paused
    );
    assert_eq!(
        store
            .get_run_progress(selected)
            .await?
            .unwrap()
            .lifecycle_state,
        gammaboard::stores::RunLifecycleState::Paused
    );
    store
        .assign_worker_pool("z-sampler", WorkerRole::SamplerAggregator, parent)
        .await?;
    store
        .assign_worker_pool("z-evaluator", WorkerRole::Evaluator, parent)
        .await?;
    harness
        .wait_for("resume child runtime", Duration::from_secs(15), || async {
            Ok(
                sqlx::query_scalar::<_, i64>("SELECT count(*) FROM nodes WHERE active_run_id=$1")
                    .bind(selected)
                    .fetch_one(&harness.pool)
                    .await?
                    == 2,
            )
        })
        .await?;
    gammaboard::api::runs::remove_run(&store, parent).await?;
    sleep(Duration::from_millis(500)).await;
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(remaining, 0);
    let errors: i64 = sqlx::query_scalar("SELECT count(*) FROM runtime_logs WHERE message LIKE '%foreign key%' OR message LIKE '%failed to stop role runner cleanly%'").fetch_one(&harness.pool).await?;
    assert_eq!(
        errors, 0,
        "active deletion must drain writers before removing rows"
    );
    harness.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn campaign_parallel_quanta_keep_other_workers_and_recover_within_shared_cap()
-> anyhow::Result<()> {
    for capped in [true, false] {
        let mut harness = FullStackHarness::new().await?;
        let stop = if capped {
            "max_total_samples = 768"
        } else {
            "absolute_error = 1e-12"
        };
        let mut card = format!(
            r#"
kind = "integration_campaign"
name = "parallel-quanta"
stop_condition = {{ {stop} }}
allocation = {{ max_active_runs = 4, min_samples_per_child = 128, allocation_window_samples = 128 }}
"#
        );
        for (name, delay, seed) in [
            ("fast-a", 0.001, 11),
            ("slow-b", 0.08, 22),
            ("fast-c", 0.001, 33),
        ] {
            card.push_str(&format!(
                r#"
[[children]]
name = "{name}"
run = '''
name = "{name}"
[evaluator]
kind = "unit"
continuous_dims = 1
timing = {{ per_sample_seconds = {delay} }}
[sampler_aggregator_runner_params]
frontend_sync_interval_ms = 20
performance_snapshot_interval_ms = 20
[sampler_aggregator_runner_params.queue]
max_batch_size = 16
max_generation_size = 4096
target_batch_eval_ms = 100
[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = {{ max_samples = 512 }}
accumulator = {{ config = "scalar" }}
sampler = {{ config = {{ kind = "naive_monte_carlo", seed = {seed} }} }}
'''
"#
            ));
        }
        harness.add_run(&temp_config(&card));
        let parent = harness.run_id("parallel-quanta").await?;
        harness
            .start_nodes(&[
                "parallel-0",
                "parallel-1",
                "parallel-2",
                "parallel-3",
                "parallel-4",
                "parallel-5",
            ])
            .await?;
        let store = gammaboard::PgStore::new(harness.pool.clone());
        let assigned = gammaboard::api::nodes::auto_assign_run(&store, parent, None).await?;
        assert_eq!(
            (
                assigned.assigned_samplers.len(),
                assigned.assigned_evaluators.len()
            ),
            (2, 4)
        );
        harness.wait_for("slow child has a complete worker bundle", Duration::from_secs(30), || async {
        Ok(sqlx::query_scalar::<_, bool>("SELECT count(*)=3 FROM nodes n JOIN runs r ON r.id=n.active_run_id WHERE r.parent_run_id=$1 AND r.spawn_label='slow-b'")
            .bind(parent).fetch_one(&harness.pool).await?)
    }).await?;
        let before: Vec<(String, String)> = sqlx::query_as("SELECT n.name,n.active_role FROM nodes n JOIN runs r ON r.id=n.active_run_id WHERE r.parent_run_id=$1 AND r.spawn_label='slow-b' ORDER BY n.name")
        .bind(parent).fetch_all(&harness.pool).await?;
        harness.wait_for("C starts while B is still inside its first quantum", Duration::from_secs(30), || async {
        Ok(sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM runs r JOIN run_tasks t ON t.run_id=r.id WHERE r.parent_run_id=$1 AND r.spawn_label='fast-c' AND t.nr_produced_samples>0) AND EXISTS(SELECT 1 FROM runs r JOIN run_tasks t ON t.run_id=r.id WHERE r.parent_run_id=$1 AND r.spawn_label='slow-b' AND t.nr_completed_samples<128)")
            .bind(parent).fetch_one(&harness.pool).await?)
    }).await?;
        let after: Vec<(String,String)> = sqlx::query_as("SELECT n.name,n.active_role FROM nodes n JOIN runs r ON r.id=n.active_run_id WHERE r.parent_run_id=$1 AND r.spawn_label='slow-b' ORDER BY n.name")
        .bind(parent).fetch_all(&harness.pool).await?;
        assert_eq!(
            before, after,
            "switching A to C must leave B's sampler and evaluators in place"
        );
        let sampler = before
            .iter()
            .find(|(_, role)| role == "sampler_aggregator")
            .unwrap()
            .0
            .clone();
        harness.kill_child(&sampler).await?;
        sqlx::query("UPDATE nodes SET lease_expires_at=now()-interval '1 second' WHERE name=$1")
            .bind(&sampler)
            .execute(&harness.pool)
            .await?;
        harness.start_node(&sampler).await?;
        wait_for_task_state(&harness, parent, "completed", Duration::from_secs(90)).await?;
        let samples: (i64,i64) = sqlx::query_as("SELECT sum(t.nr_produced_samples)::bigint,sum(t.nr_completed_samples)::bigint FROM runs r JOIN run_tasks t ON t.run_id=r.id WHERE r.parent_run_id=$1")
        .bind(parent).fetch_one(&harness.pool).await?;
        assert_eq!(samples.0, samples.1);
        assert!(samples.0 >= 384 && samples.0 <= 1536);
        if capped {
            assert_eq!(samples.0, 768);
        }
        let grants: i64 = sqlx::query_scalar("SELECT sum(granted_samples)::bigint FROM campaign_sample_budgets b JOIN run_tasks t ON t.id=b.campaign_task_id WHERE t.run_id=$1")
        .bind(parent).fetch_one(&harness.pool).await?;
        assert_eq!(grants, samples.0);
        let result: JsonValue =
            sqlx::query_scalar("SELECT measurement_output FROM run_tasks WHERE run_id=$1")
                .bind(parent)
                .fetch_one(&harness.pool)
                .await?;
        assert_eq!(result["results"][0]["value"], 3.0);
        assert_eq!(result["results"][0]["sample_count"], samples.0);
        let saved: Vec<(JsonValue, i64)> = sqlx::query_as("SELECT c.sampler_checkpoint,t.nr_completed_samples FROM run_sampler_checkpoints c JOIN runs r ON r.id=c.run_id JOIN run_tasks t ON t.id=c.task_id WHERE r.parent_run_id=$1")
            .bind(parent).fetch_all(&harness.pool).await?;
        assert_eq!(saved.len(), 3);
        for (checkpoint, count) in saved {
            let saved: gammaboard::SavedSampler =
                serde_json::from_value(checkpoint["sampler_snapshot"].clone())?;
            let mut direct = saved.config.create(
                gammaboard::Domain::continuous(1),
                Some(count as usize),
                json!({}),
            )?;
            direct.generate(count as usize)?;
            let expected = direct.snapshot()?;
            assert_eq!(saved.state["produced_samples"], count);
            assert_eq!(
                saved.state["rng"], expected["rng"],
                "switching/recovery must preserve the uninterrupted RNG stream"
            );
        }
        harness.cleanup().await?;
    }
    Ok(())
}
