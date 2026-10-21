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
        Ok(sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM run_sampler_checkpoints c JOIN run_tasks t ON t.id=c.task_id WHERE c.run_id=$1 AND c.sampler_checkpoint->'observable_state'->>'kind'='stratified' AND t.nr_completed_samples BETWEEN 64 AND 8192 AND t.state='active')").bind(run).fetch_one(&harness.pool).await?)
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
    let saved_count = checkpoint["completed_samples"]
        .as_i64()
        .context("checkpoint sample count")?;
    anyhow::ensure!(
        (64..16384).contains(&saved_count),
        "pause must interrupt inference, saved {saved_count}"
    );
    let proposal = checkpoint["observable_state"]["plan"]["proposal_id"]
        .as_str()
        .context("missing frozen proposal")?
        .to_string();
    harness
        .cli()
        .args(["node", "auto-assign", &run.to_string()])
        .assert()
        .success();
    wait_stratified_completion(&harness, run).await?;
    let state = load_stratified_result(&harness, run, "infer").await?;
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
    wait_stratified_completion(&harness, run).await?;
    let state = load_stratified_result(&harness, run, "infer").await?;
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

async fn wait_stratified_completion(harness: &FullStackHarness, run: i32) -> anyhow::Result<()> {
    harness
        .wait_for(
            "stratified tasks complete",
            Duration::from_secs(90),
            || async {
                let rows: Vec<(String, Option<String>)> =
                    sqlx::query_as("SELECT state,failure_reason FROM run_tasks WHERE run_id=$1")
                        .bind(run)
                        .fetch_all(&harness.pool)
                        .await?;
                anyhow::ensure!(
                    !rows.iter().any(|(state, _)| state == "failed"),
                    "task failed: {rows:?}"
                );
                Ok(rows.iter().all(|(state, _)| state == "completed"))
            },
        )
        .await
}

async fn load_stratified_result(
    harness: &FullStackHarness,
    run: i32,
    task: &str,
) -> anyhow::Result<gammaboard::evaluation::accumulator::StratifiedAccumulatorState> {
    let value: JsonValue = sqlx::query_scalar(
        "SELECT s.observable_state FROM run_stage_snapshots s JOIN run_tasks t ON t.id=s.task_id
         WHERE t.run_id=$1 AND t.name=$2 ORDER BY s.id DESC LIMIT 1",
    )
    .bind(run)
    .bind(task)
    .fetch_one(&harness.pool)
    .await?;
    let gammaboard::evaluation::AccumulatorState::Stratified(state) =
        gammaboard::evaluation::AccumulatorState::from_json(&value)?
    else {
        anyhow::bail!("expected stratified state")
    };
    Ok(*state)
}

/// Independently recompute raw moments from the reference sample stream. This
/// detects duplication/loss and incorrect weights without using accumulator merge.
fn assert_strata_match_reference(
    state: &gammaboard::evaluation::accumulator::StratifiedAccumulatorState,
    reference: &[Vec<(f64, bool)>],
) {
    assert_eq!(state.strata.len(), reference.len());
    for (stratum, samples) in state.strata.iter().zip(reference) {
        assert_eq!(stratum.attempts, samples.len() as u64);
        assert_eq!(
            stratum.matches,
            samples.iter().filter(|(_, hit)| *hit).count() as u64
        );
        let mean = samples.iter().map(|(value, _)| value).sum::<f64>() / samples.len() as f64;
        let m2 = samples
            .iter()
            .map(|(value, _)| (value - mean).powi(2))
            .sum::<f64>();
        assert!(
            (stratum.mean[0] - mean).abs() < 1e-11 * mean.abs().max(1.0),
            "{} != {mean}",
            stratum.mean[0]
        );
        assert!(
            (stratum.m2[0] - m2).abs() < 1e-10 * m2.abs().max(1.0),
            "{} != {m2}",
            stratum.m2[0]
        );
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires disposable postgres with CREATE DATABASE privilege"]
async fn stratified_sigkill_with_outstanding_work_replays_the_frozen_sample_stream()
-> anyhow::Result<()> {
    use gammaboard::{
        core::SamplingConfig,
        sampling::{Generation, SavedSampler, StageHandoff},
    };
    let mut harness = FullStackHarness::new().await?;
    let config = temp_config(
        r#"
name="stratified-crash"
[evaluator]
kind="unit"
continuous_dims=1
value_coordinate=0
timing={overhead_seconds=0.02}
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
sampler={config={kind="havana_training",seed=17,bins=8,samples_for_update=128,stratification={kind="voronoi",centers=[[0.25],[0.75]]}}}
[[task_queue]]
name="infer"
kind="sample"
estimator="stratified"
stop_condition={max_samples=8192}
accumulator={config="scalar"}
sampler={config={kind="havana_inference",seed=18},from_stage="train"}
"#,
    );
    harness.add_run(&config);
    let run = harness.run_id("stratified-crash").await?;
    harness.start_nodes(&["crash-s", "crash-e"]).await?;
    harness.assign_node("crash-s", "sampler_aggregator", "stratified-crash");
    harness.assign_node("crash-e", "evaluator", "stratified-crash");
    harness.wait_for("inference checkpoint and outstanding claims",Duration::from_secs(60),|| async {
        Ok(sqlx::query_scalar::<_,bool>(
            "SELECT EXISTS(SELECT 1 FROM run_sampler_checkpoints c JOIN run_tasks t ON t.id=c.task_id
             WHERE c.run_id=$1 AND t.name='infer' AND t.nr_completed_samples BETWEEN 64 AND 4096 AND t.state='active'
             AND EXISTS(SELECT 1 FROM batches b WHERE b.task_id=t.id AND b.status='claimed'))"
        ).bind(run).fetch_one(&harness.pool).await?)
    }).await?;
    harness.signal_child("crash-e", libc::SIGSTOP)?;
    harness.signal_child("crash-s", libc::SIGSTOP)?;
    let before = harness
        .run_sampler_checkpoint(run)
        .await?
        .context("checkpoint")?;
    let plan = before["observable_state"]["plan"].clone();
    let outstanding:i64=sqlx::query_scalar("SELECT count(*) FROM batches b JOIN run_tasks t ON t.id=b.task_id WHERE b.run_id=$1 AND t.name='infer' AND b.status IN ('pending','claimed')")
        .bind(run).fetch_one(&harness.pool).await?;
    anyhow::ensure!(outstanding > 0, "crash did not interrupt queued work");
    harness.kill_child("crash-e").await?;
    harness.kill_child("crash-s").await?;
    // Expedite lease expiry only after the processes have actually died.
    harness.expire_node("crash-e").await?;
    harness.expire_node("crash-s").await?;
    harness.start_nodes(&["crash-s", "crash-e"]).await?;
    harness
        .cli()
        .args(["node", "auto-assign", &run.to_string()])
        .assert()
        .success();
    wait_stratified_completion(&harness, run).await?;
    let state = load_stratified_result(&harness, run, "infer").await?;
    assert_eq!(serde_json::to_value(&state.plan)?, plan);
    assert_eq!(state.sample_count(), 8192);
    let counts:Vec<(i64,i64)>=sqlx::query_as("SELECT nr_produced_samples,nr_completed_samples FROM run_tasks WHERE run_id=$1 ORDER BY sequence_nr")
        .bind(run).fetch_all(&harness.pool).await?;
    assert_eq!(counts, vec![(512, 512), (8192, 8192)]);
    let value:JsonValue=sqlx::query_scalar("SELECT s.sampler_snapshot FROM run_stage_snapshots s JOIN run_tasks t ON t.id=s.task_id WHERE t.run_id=$1 AND t.name='train' ORDER BY s.id DESC LIMIT 1")
        .bind(run).fetch_one(&harness.pool).await?;
    let saved: SavedSampler = serde_json::from_value(value)?;
    let config: SamplingConfig =
        serde_json::from_value(json!({"kind":"havana_inference","seed":18}))?;
    let config = config.inherit_stratification(&saved)?;
    let domain = gammaboard::Domain::continuous(1);
    let source = StageHandoff {
        sampler_snapshot: Some(&saved),
        observable_state: None,
    };
    let mut sampler = config.from_stage(&domain, source)?;
    let mut materializer = config.build_materializer(&domain, Some(source))?;
    let mut reference = vec![vec![], vec![]];
    let mut remaining = 8192;
    while remaining > 0 {
        let Generation::Batch { batch, .. } = sampler.generate(remaining.min(79))? else {
            anyhow::bail!("reference inference stopped")
        };
        remaining -= batch.nr_samples;
        for point in materializer.materialize_batch(&batch)?.points() {
            let k = point.discrete[0] as usize;
            let x = point.continuous[0];
            let hit = k == usize::from(x > 0.5);
            reference[k].push((
                if hit {
                    x * point.total_weight() * state.plan.probabilities.as_ref().unwrap()[k]
                } else {
                    0.
                },
                hit,
            ));
        }
    }
    assert_strata_match_reference(&state, &reference);
    harness.cleanup().await
}

#[tokio::test]
#[ignore = "requires disposable postgres and numpy"]
async fn stratified_process_materializer_and_transform_preserve_labels_and_jacobians()
-> anyhow::Result<()> {
    use gammaboard::{core::SamplingConfig, sampling::Generation};
    let mut harness = FullStackHarness::new().await?;
    let python = std::env::var("GAMMABOARD_PROCESS_PYTHON").unwrap_or_else(|_| "python3".into());
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stratified_pipeline.py");
    let command = |role: &str| {
        serde_json::to_string(&vec![
            python.clone(),
            script.display().to_string(),
            role.into(),
        ])
        .unwrap()
    };
    let materializer = command("materializer");
    let transform = command("transform");
    let evaluator = command("evaluator");
    let config = temp_config(&format!(
        r#"
name="stratified-pipeline"
[evaluator]
kind="process_evaluator"
command={evaluator}
domain={{rectangular={{discrete_cardinalities=[2],continuous_dims=1}}}}
output={{kind="values",components=["value"]}}
[sampler_aggregator_runner_params.queue]
fixed_batch_size=137
max_generation_size=509
[[task_queue]]
name="infer"
kind="sample"
estimator="stratified"
stop_condition={{max_samples=16384}}
accumulator={{config="scalar"}}
[task_queue.sampler.config]
kind="naive_monte_carlo"
seed=29
materializer={{kind="process_materializer",command={materializer}}}
transforms=[{{kind="process_batch_transform",command={transform}}}]
stratification={{kind="voronoi",centers=[[2.0],[10.0]],scales=[3.0]}}
"#
    ));
    harness.add_run(&config);
    let run = harness.run_id("stratified-pipeline").await?;
    harness.start_nodes(&["pipe-s", "pipe-e"]).await?;
    harness.assign_node("pipe-s", "sampler_aggregator", "stratified-pipeline");
    harness.assign_node("pipe-e", "evaluator", "stratified-pipeline");
    wait_stratified_completion(&harness, run).await?;
    let state = load_stratified_result(&harness, run, "infer").await?;
    assert_eq!(state.sample_count(), 16384);
    assert_eq!(state.plan.probabilities, Some(vec![0.5, 0.5]));
    let config: SamplingConfig = serde_json::from_value(
        json!({"kind":"naive_monte_carlo","seed":29,"stratification":{"kind":"voronoi","centers":[[2.],[10.]],"scales":[3.]}}),
    )?;
    let mut sampler = config.create(
        gammaboard::Domain::rectangular_with_cardinalities(1, [2]),
        None,
        json!({}),
    )?;
    let mut reference = vec![vec![], vec![]];
    let mut remaining = 16384;
    while remaining > 0 {
        let Generation::Batch { batch, .. } = sampler.generate(remaining.min(211))? else {
            anyhow::bail!("reference stopped")
        };
        remaining -= batch.nr_samples;
        for point in batch.payload.into_batch()?.points() {
            let k = point.discrete[0] as usize;
            let d = point.discrete[1];
            let u = point.continuous[0];
            let value = (d + 1) as f64 * (6. * u + 1.);
            let hit = k == usize::from(value > 6.);
            // The expected conditional weight is 1/p(d) * dz/du * dx/dz = 12.
            reference[k].push((if hit { 12. * value } else { 0. }, hit));
        }
    }
    assert_strata_match_reference(&state, &reference);
    for (k, exact) in [25.5, 46.5].into_iter().enumerate() {
        assert!(
            (state.strata[k].mean[0] - exact).abs()
                < 6. * state.strata[k].standard_error(0).unwrap()
        );
    }
    assert!((state.mean().unwrap()[0] - 72.).abs() < 6. * state.covariance().unwrap()[0].sqrt());
    harness.cleanup().await
}
