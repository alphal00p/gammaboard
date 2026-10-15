use super::*;

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn failure_policy() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness.start_nodes(&["s", "e"]).await?;
    // One batch isolates retry limits from scheduling. Every case starts fresh
    // engines, but shares the database and idle worker processes.
    for (name, evaluator, sampler, retries, failure) in [
        ("sampler", "", ", fail_on_produce_batch_nr = 1", 0, Some("")),
        (
            "materializer",
            "",
            ", fail_on_materialize_batch_nr = 1",
            1,
            None,
        ),
        ("evaluate-once", "fail_on_batch_nrs = [1]", "", 1, None),
        ("evaluate-twice", "fail_on_batch_nrs = [1,2]", "", 2, None),
        (
            "evaluate-exhausted",
            "fail_on_batch_nrs = [1,2,3]",
            "",
            3,
            Some("3/3"),
        ),
        (
            "build",
            "fail_on_build = true",
            "",
            0,
            Some("failed to build evaluator"),
        ),
    ] {
        eprintln!("failure policy: {name}");
        harness.add_run(&temp_config(&format!(
            r#"
name = "{name}"
[evaluator]
kind = "unit"
{evaluator}
[sampler_aggregator_runner_params.queue]
fixed_batch_size = 32
max_batch_retries = 3
[[task_queue]]
kind = "sample"
stop_condition = {{ max_samples = 32 }}
accumulator = {{ config = "scalar" }}
sampler = {{ config = {{ kind = "naive_monte_carlo" {sampler} }} }}
"#
        )));
        let run = harness.run_id(name).await?;
        harness.assign_node("s", "sampler_aggregator", name);
        harness.assign_node("e", "evaluator", name);
        if retries > 0 {
            wait_for_batch_retry_count(&harness, run, retries, Duration::from_secs(30)).await?;
        }
        if let Some(reason) = failure {
            wait_for_task_failed_and_run_unassigned(&harness, run, Duration::from_secs(30)).await?;
            let actual: String =
                sqlx::query_scalar("SELECT failure_reason FROM run_tasks WHERE run_id=$1")
                    .bind(run)
                    .fetch_one(&harness.pool)
                    .await?;
            anyhow::ensure!(
                !actual.is_empty() && actual.contains(reason),
                "{name}: {actual}"
            );
            if retries == 3 {
                wait_for_failed_batch(&harness, run, Duration::from_secs(10)).await?;
            }
        } else {
            wait_for_task_state(&harness, run, "completed", Duration::from_secs(30)).await?;
            anyhow::ensure!(
                harness.run_sample_progress(run).await? == (32, 32),
                "{name}: retry changed counts"
            );
            let failed: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM batches WHERE run_id=$1 AND status='failed'",
            )
            .bind(run)
            .fetch_one(&harness.pool)
            .await?;
            anyhow::ensure!(
                failed == 0,
                "{name}: recovered batch left permanently failed"
            );
        }
    }
    harness.cleanup().await
}

#[tokio::test]
#[ignore = "requires PostgreSQL and Python with NumPy"]
async fn invalid_results_publish_neither_progress_nor_feedback() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness.start_nodes(&["s", "e"]).await?;
    let python = std::env::var("GAMMABOARD_PROCESS_PYTHON").unwrap_or_else(|_| "python3".into());
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/invalid_evaluator.py");
    for process in [false, true] {
        for training in [false, true] {
            let name = format!("invalid-{process}-{training}");
            let evaluator = if process {
                format!(
                    r#"kind = "process_evaluator"
command = [{python:?}, {fixture:?}]
domain = {{ rectangular = {{ continuous_dims = 1, discrete_cardinalities = [] }} }}"#
                )
            } else {
                "kind = 'symbolica'\nsource = {kind='expression',expr='10^200'}\nargs = ['x']"
                    .into()
            };
            let accumulator = if process {
                r#"{ kind = "vector", components = ["value"], training_projection = { kind = "component", name = "value" } }"#
            } else {
                "'scalar'"
            };
            let window = if training { 4 } else { 0 };
            harness.add_run(&temp_config(&format!(
                r#"
name = "{name}"
[evaluator]
{evaluator}
[sampler_aggregator_runner_params.queue]
fixed_batch_size = 4
max_batch_retries = 1
[[task_queue]]
kind = "sample"
stop_condition = {{ max_samples = 4 }}
accumulator = {{ config = {accumulator} }}
sampler = {{ config = {{ kind = "naive_monte_carlo", training_window_samples = {window} }} }}
"#
            )));
            let run = harness.run_id(&name).await?;
            harness.assign_node("s", "sampler_aggregator", &name);
            harness.assign_node("e", "evaluator", &name);
            wait_for_task_failed_and_run_unassigned(&harness, run, Duration::from_secs(30)).await?;
            let reason: String =
                sqlx::query_scalar("SELECT failure_reason FROM run_tasks WHERE run_id=$1")
                    .bind(run)
                    .fetch_one(&harness.pool)
                    .await?;
            anyhow::ensure!(
                reason.contains("non-finite second moment"),
                "{name}: {reason}"
            );
            anyhow::ensure!(
                harness.run_sample_progress(run).await?.1 == 0,
                "invalid batch advanced progress"
            );
            let accepted: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM batches WHERE run_id=$1 AND status='completed'",
            )
            .bind(run)
            .fetch_one(&harness.pool)
            .await?;
            anyhow::ensure!(accepted == 0, "invalid batch was published");
            let checkpoint = harness
                .run_sampler_checkpoint(run)
                .await?
                .expect("initial checkpoint");
            anyhow::ensure!(
                checkpoint["sampler_snapshot"]["state"]["returned_samples"] == 0,
                "invalid feedback reached sampler: {checkpoint}"
            );
        }
    }
    harness.cleanup().await
}
