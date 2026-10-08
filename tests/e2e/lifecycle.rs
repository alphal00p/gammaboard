use super::*;

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn worker_lifecycle() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness.start_nodes(&["s", "e"]).await?;
    // Empty runs clear every role, including repeated assignments after pause.
    harness.add_run(&temp_config("name='idle'"));
    for role in ["evaluator", "sampler_aggregator"] {
        harness.assign_node("s", role, "idle");
        harness
            .wait_for("idle role released", Duration::from_secs(10), || async {
                Ok(harness.node_state("s").await? == (None, None, None, None))
            })
            .await?;
        harness
            .cli()
            .args(["run", "pause", "idle"])
            .assert()
            .success();
    }
    harness
        .cli()
        .args(["node", "assign", "missing", "evaluator", "idle"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not live"));
    harness
        .cli()
        .args(["node", "assign", "s", "evaluator", "99999"])
        .assert()
        .failure();
    harness
        .cli()
        .args(["node", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("N/A"));
    harness
        .cli()
        .arg("run")
        .arg("create")
        .arg(temp_config("name='invalid'\n[point_spec]\ncontinuous_dims=1").path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("point_spec"));
    harness.add_run(&temp_config(
        r#"
name = "lifecycle"
[evaluator]
kind = "unit"
timing = { overhead_seconds = 0.05 }
[sampler_aggregator_runner_params]
frontend_sync_interval_ms = 20
[sampler_aggregator_runner_params.queue]
fixed_batch_size = 32
max_generation_size = 1024
[[task_queue]]
kind = "sample"
stop_condition = { max_samples = 100000 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"#,
    ));
    let run = harness.run_id("lifecycle").await?;
    harness.assign_node("s", "sampler_aggregator", "lifecycle");
    harness.assign_node("e", "evaluator", "lifecycle");
    harness
        .wait_for("live processing", Duration::from_secs(15), || async {
            Ok(harness.run_sample_progress(run).await?.1 >= 32)
        })
        .await?;
    // Graceful evaluator termination releases the lease/name immediately.
    harness.terminate_child("e").await?;
    assert_eq!(harness.node_state("e").await?, (None, None, None, None));
    let leased: bool =
        sqlx::query_scalar("SELECT lease_expires_at>now() FROM nodes WHERE name='e'")
            .fetch_one(&harness.pool)
            .await?;
    assert!(!leased);
    harness.start_node("e").await?;
    harness.assign_node("e", "evaluator", "lifecycle");
    let store = gammaboard::PgStore::new(harness.pool.clone());
    let result = node_api::stop_all_nodes_gracefully(
        &store,
        node_api::GracefulNodeShutdownParams {
            sampler_drain_timeout_seconds: 10,
            node_stop_timeout_seconds: 10,
            poll_interval_ms: 25,
        },
    )
    .await?;
    assert!(!result.sampler_drain_timed_out);
    assert!(result.assignments_cleared >= 2);
    assert_eq!(result.active_samplers_remaining, 0);
    harness.reap_children(&["s", "e"]).await?;
    let saved = harness.run_sampler_checkpoint(run).await?.unwrap();
    let count = saved["completed_samples"].as_i64().unwrap();
    assert!(count >= 32);
    let stages = harness.run_stage_snapshot_count(run).await?;
    let outputs = harness.persisted_observable_snapshot_count(run).await?;
    harness.start_nodes(&["s", "e"]).await?;
    harness.assign_node("s", "sampler_aggregator", "lifecycle");
    harness.assign_node("e", "evaluator", "lifecycle");
    harness
        .wait_for(
            "progress after graceful restart",
            Duration::from_secs(15),
            || async { Ok(harness.run_sample_progress(run).await?.1 > count) },
        )
        .await?;
    assert!(harness.run_stage_snapshot_count(run).await? >= stages);
    assert!(harness.persisted_observable_snapshot_count(run).await? >= outputs);
    harness
        .cli()
        .args(["run", "remove", "--yes", "lifecycle"])
        .assert()
        .success();
    harness
        .wait_for(
            "active removal clears roles",
            Duration::from_secs(15),
            || async {
                Ok(sqlx::query_scalar::<_, i64>(
                    "SELECT count(*) FROM nodes WHERE active_run_id=$1 OR desired_run_id=$1",
                )
                .bind(run)
                .fetch_one(&harness.pool)
                .await?
                    == 0)
            },
        )
        .await?;
    harness.cleanup().await
}
