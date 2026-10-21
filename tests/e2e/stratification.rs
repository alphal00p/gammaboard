use super::*;
use anyhow::Context;

#[tokio::test]
#[ignore = "requires disposable postgres with CREATE DATABASE privilege"]
async fn stratified_havana_queue_and_checkpoint_resume_preserve_counts_and_plan()
-> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    let config = temp_config(
        r#"
name="stratified-recovery"
[evaluator]
kind="unit"
continuous_dims=1
value_coordinate=0
timing={per_sample_seconds=0.0002}
[sampler_aggregator_runner_params]
frontend_sync_interval_ms=50
min_tick_time_ms=1
[sampler_aggregator_runner_params.queue]
fixed_batch_size=31
max_generation_size=113
[[task_queue]]
name="train"
kind="sample"
publish_result=false
stop_condition={max_samples=512}
accumulator={config="scalar"}
sampler={config={kind="havana_training",seed=7,bins=8,samples_for_update=128,stratification={kind="voronoi",centers=[[0.25],[0.75]],scales=[2.0]}}}
[[task_queue]]
name="infer"
kind="sample"
estimator="stratified"
stop_condition={max_samples=16384}
accumulator={config="scalar"}
sampler={config={kind="havana_inference",seed=8},from_stage="train"}
"#,
    );
    harness.add_run(&config);
    let run = harness.run_id("stratified-recovery").await?;
    harness.start_nodes(&["strat-s", "strat-e"]).await?;
    harness.assign_node("strat-s", "sampler_aggregator", "stratified-recovery");
    harness.assign_node("strat-e", "evaluator", "stratified-recovery");
    harness.wait_for("stratified inference checkpoint",Duration::from_secs(60),|| async {
        let failed:Option<String>=sqlx::query_scalar("SELECT failure_reason FROM run_tasks WHERE run_id=$1 AND state='failed' LIMIT 1").bind(run).fetch_optional(&harness.pool).await?.flatten();
        anyhow::ensure!(failed.is_none(),"task failed: {failed:?}");
        Ok(sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM run_sampler_checkpoints WHERE run_id=$1 AND sampler_checkpoint->'observable_state'->>'kind'='stratified' AND (sampler_checkpoint->>'completed_samples')::bigint>0)").bind(run).fetch_one(&harness.pool).await?)
    }).await?;
    harness
        .cli()
        .args(["run", "pause", &run.to_string()])
        .assert()
        .success();
    harness
        .wait_for(
            "workers detached after pause",
            Duration::from_secs(30),
            || async {
                Ok(sqlx::query_scalar::<_, i64>(
                    "SELECT count(*) FROM nodes WHERE active_run_id=$1",
                )
                .bind(run)
                .fetch_one(&harness.pool)
                .await?
                    == 0)
            },
        )
        .await?;
    let checkpoint: JsonValue = sqlx::query_scalar(
        "SELECT sampler_checkpoint FROM run_sampler_checkpoints WHERE run_id=$1",
    )
    .bind(run)
    .fetch_one(&harness.pool)
    .await?;
    let proposal = checkpoint["observable_state"]["plan"]["proposal_id"]
        .as_str()
        .context("missing frozen proposal")?
        .to_string();
    harness
        .cli()
        .args(["node", "auto-assign", &run.to_string()])
        .assert()
        .success();
    harness
        .wait_for(
            "stratified task complete",
            Duration::from_secs(60),
            || async {
                let rows: Vec<(String, Option<String>)> =
                    sqlx::query_as("SELECT state,failure_reason FROM run_tasks WHERE run_id=$1")
                        .bind(run)
                        .fetch_all(&harness.pool)
                        .await?;
                anyhow::ensure!(
                    !rows.iter().any(|r| r.0 == "failed"),
                    "task failed: {rows:?}"
                );
                Ok(rows.iter().all(|r| r.0 == "completed"))
            },
        )
        .await?;
    let state:JsonValue=sqlx::query_scalar("SELECT s.observable_state FROM run_stage_snapshots s JOIN run_tasks t ON t.id=s.task_id WHERE t.run_id=$1 AND t.name='infer' ORDER BY s.id DESC LIMIT 1").bind(run).fetch_one(&harness.pool).await?;
    let gammaboard::evaluation::AccumulatorState::Stratified(state) =
        gammaboard::evaluation::AccumulatorState::from_json(&state)?
    else {
        anyhow::bail!("expected stratified state")
    };
    assert_eq!(state.sample_count(), 16384);
    assert_eq!(state.plan.proposal_id, proposal);
    assert!(state.strata.iter().any(|s| s.matches < s.attempts));
    let mean = state.mean().unwrap()[0];
    let error = state.covariance().unwrap()[0].sqrt();
    assert!((mean - 0.5).abs() < 6. * error, "{mean} +/- {error}");
    let counts:Vec<(i64,i64)>=sqlx::query_as("SELECT nr_produced_samples,nr_completed_samples FROM run_tasks WHERE run_id=$1 ORDER BY sequence_nr").bind(run).fetch_all(&harness.pool).await?;
    assert_eq!(counts, vec![(512, 512), (16384, 16384)]);
    harness.cleanup().await
}

#[tokio::test]
#[ignore = "requires disposable postgres and numpy"]
async fn stratified_process_stage_handoff_uses_exact_probabilities() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    let python = std::env::var("GAMMABOARD_PROCESS_PYTHON").unwrap_or_else(|_| "python3".into());
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stratified_sampler.py");
    let command = serde_json::to_string(&vec![python, script.display().to_string()])?;
    let config = temp_config(&format!(
        r#"
name="process-stratified"
[evaluator]
kind="unit"
continuous_dims=1
value_coordinate=0
[[task_queue]]
name="train"
kind="sample"
publish_result=false
stop_condition={{max_samples=512}}
accumulator={{config="scalar"}}
sampler={{config={{kind="process_sampler",command={command},args={{inference=false}},stratification={{kind="voronoi",centers=[[0.25],[0.75]]}}}}}}
[[task_queue]]
name="infer"
kind="sample"
estimator="stratified"
stop_condition={{max_samples=8192}}
accumulator={{config="scalar"}}
sampler={{config={{kind="process_sampler",command={command},args={{inference=true,seed=9}}}},from_stage="train"}}
"#
    ));
    harness.add_run(&config);
    let run = harness.run_id("process-stratified").await?;
    harness.start_nodes(&["process-s", "process-e"]).await?;
    harness.assign_node("process-s", "sampler_aggregator", "process-stratified");
    harness.assign_node("process-e", "evaluator", "process-stratified");
    harness
        .wait_for(
            "process stratified inference complete",
            Duration::from_secs(60),
            || async {
                let rows: Vec<(String, Option<String>)> =
                    sqlx::query_as("SELECT state,failure_reason FROM run_tasks WHERE run_id=$1")
                        .bind(run)
                        .fetch_all(&harness.pool)
                        .await?;
                anyhow::ensure!(
                    !rows.iter().any(|r| r.0 == "failed"),
                    "task failed: {rows:?}"
                );
                Ok(rows.iter().all(|r| r.0 == "completed"))
            },
        )
        .await?;
    let state:JsonValue=sqlx::query_scalar("SELECT s.observable_state FROM run_stage_snapshots s JOIN run_tasks t ON t.id=s.task_id WHERE t.run_id=$1 AND t.name='infer' ORDER BY s.id DESC LIMIT 1").bind(run).fetch_one(&harness.pool).await?;
    let gammaboard::evaluation::AccumulatorState::Stratified(state) =
        gammaboard::evaluation::AccumulatorState::from_json(&state)?
    else {
        anyhow::bail!("expected stratified state")
    };
    assert_eq!(state.sample_count(), 8192);
    assert_eq!(state.plan.probabilities, Some(vec![0.3, 0.7]));
    assert!((state.mean().unwrap()[0] - 0.5).abs() < 6. * state.covariance().unwrap()[0].sqrt());
    harness.cleanup().await
}

#[test]
fn scaled_voronoi_run_cards_validate_and_reject_invalid_estimators() {
    for card in [
        include_str!("../../resources/templates/runs/tetrahedron-stratified.toml"),
        include_str!("../../resources/templates/runs/tetrahedron-ordinary.toml"),
    ] {
        let definition = gammaboard::api::runs::parse_run_add_config_toml(card).unwrap();
        assert_eq!(
            gammaboard::api::runs::validate_run(definition, true)
                .unwrap()
                .tasks_validated,
            2
        );
    }
    let card = include_str!("../../resources/templates/runs/tetrahedron-stratified.toml");
    let invalid = card.replace("scales = [1.0, 1.0, 1.0]", "scales = [0.0, 1.0, 1.0]");
    let definition = gammaboard::api::runs::parse_run_add_config_toml(&invalid).unwrap();
    assert!(gammaboard::api::runs::validate_run(definition, false).is_err());
    let invalid=card.replace("kind = \"vector\", components = [\"x\", \"y\", \"z\"], training_projection = { kind = \"norm\" }","kind = \"gammaloop\"");
    let definition = gammaboard::api::runs::parse_run_add_config_toml(&invalid).unwrap();
    assert!(gammaboard::api::runs::validate_run(definition, false).is_err());
}
