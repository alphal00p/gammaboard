//! Seeded, real-process fault sequences. See docs/recovery-testing.md for the contract.
use super::*;
use gammaboard::core::{SamplerAggregatorCheckpoint, SamplerQueueTuning};
use gammaboard::evaluation::AccumulatorState;
use rand::{Rng, seq::SliceRandom};

const SAMPLES: i64 = 16381;

#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Pause,
    KillSampler,
    KillEvaluator,
    RestartFleet,
    DisconnectDatabase,
    Tune,
    StaleEvaluator,
    AbortCheckpoint,
}

/// Dyadic values make exact equality valid, independently of floating-point reduction order.
fn check_prefix(value: &JsonValue, offset: i64) -> anyhow::Result<usize> {
    let AccumulatorState::FullVector(state) = AccumulatorState::from_json(value)? else {
        anyhow::bail!("expected the full-vector recovery oracle");
    };
    anyhow::ensure!(state.invalid_entries.is_empty(), "invalid accepted samples");
    anyhow::ensure!(
        state.values_row_major.len() <= SAMPLES as usize,
        "sample budget exceeded"
    );
    for (index, value) in state.values_row_major.iter().enumerate() {
        anyhow::ensure!(
            *value == (offset + index as i64 + 1) as f64 / 32768.0,
            "accepted sample {index}: {value}; missing, duplicate, reordered, cross-run or incorrectly weighted contribution"
        );
    }
    Ok(state.values_row_major.len())
}

#[test]
fn exact_oracle_rejects_corruption() {
    let original = json!({"kind":"full_vector", "components":["value"],
        "invalid_entries":[], "values_row_major":[1.0/32768.0,2.0/32768.0,3.0/32768.0]});
    assert_eq!(check_prefix(&original, 0).unwrap(), 3);
    for corrupted in [
        json!([1.0 / 32768.0, 1.0 / 32768.0, 3.0 / 32768.0]), // duplication
        json!([1.0 / 32768.0, 3.0 / 32768.0]),                // missing contribution
        json!([2.0 / 32768.0, 1.0 / 32768.0, 3.0 / 32768.0]), // ordering
        json!([1.0 / 65536.0, 2.0 / 65536.0, 3.0 / 65536.0]), // lost weights
    ] {
        let mut value = original.clone();
        value["values_row_major"] = corrupted;
        assert!(check_prefix(&value, 0).is_err());
    }
    assert!(check_prefix(&original, 4096).is_err()); // another run's values
}

struct Recovery {
    harness: FullStackHarness,
    run: i32,
    task: i64,
    offset: i64,
    training: bool,
    trace: Vec<JsonValue>,
    checks: usize,
}

impl Recovery {
    async fn new(seed: u64, training: bool) -> anyhow::Result<Self> {
        let mut harness = FullStackHarness::new().await?;
        let python =
            std::env::var("GAMMABOARD_PROCESS_PYTHON").unwrap_or_else(|_| "python3".into());
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/recovery_sampler.py");
        let offset = (seed % 16) as i64 * 4096;
        let config = format!(
            r#"
name = "recovery"
[evaluator]
kind = "unit"
value_coordinate = 0
timing = {{ overhead_seconds = 0.04, sigma_overhead_seconds = 0.025, seed = {seed} }}
[sampler_aggregator_runner_params]
min_tick_time_ms = 20
frontend_sync_interval_ms = 20
performance_snapshot_interval_ms = 20
[sampler_aggregator_runner_params.queue]
fixed_batch_size = 17
max_batch_size = 63
max_generation_size = 1021
max_batches_per_tick = 1
completed_batch_fetch_limit = 2
[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = {{ max_samples = {SAMPLES} }}
accumulator = {{ config = "full_vector" }}
[task_queue.sampler_aggregator.config]
kind = "process_sampler"
command = [{python:?}, "-u", {fixture:?}]
requires_training_values = {training}
args = {{ training = {training}, window = 2047, offset = {offset} }}
"#
        );
        std::fs::write(harness.artifacts.join("run.toml"), &config)?;
        harness.add_run(&temp_config(&config));
        let run = harness.run_id("recovery").await?;
        let task = sqlx::query_scalar("SELECT id FROM run_tasks WHERE run_id=$1")
            .bind(run)
            .fetch_one(&harness.pool)
            .await?;
        harness.start_nodes(&["s", "e0", "e1"]).await?;
        let test = Self {
            harness,
            run,
            task,
            offset,
            training,
            trace: vec![],
            checks: 0,
        };
        test.resume().await?;
        test.wait_progress(7).await?;
        Ok(test)
    }

    fn event(&mut self, event: JsonValue) -> anyhow::Result<()> {
        self.trace.push(event);
        std::fs::write(
            self.harness.artifacts.join("trace.json"),
            serde_json::to_vec_pretty(&self.trace)?,
        )?;
        Ok(())
    }

    async fn resume(&self) -> anyhow::Result<()> {
        self.harness
            .assign_node("s", "sampler_aggregator", "recovery");
        for node in ["e0", "e1"] {
            self.harness.assign_node(node, "evaluator", "recovery");
        }
        Ok(())
    }

    async fn wait_progress(&self, completed: i64) -> anyhow::Result<()> {
        self.harness
            .wait_for(
                format!("at least {completed} samples"),
                Duration::from_secs(30),
                || async {
                    let (state, failure, count): (String, Option<String>, i64) = sqlx::query_as(
                "SELECT state,failure_reason,nr_completed_samples FROM run_tasks WHERE id=$1")
                .bind(self.task).fetch_one(&self.harness.pool).await?;
                    anyhow::ensure!(state != "failed", "recovery task failed: {failure:?}");
                    Ok(count >= completed)
                },
            )
            .await
    }

    async fn pause(&mut self) -> anyhow::Result<JsonValue> {
        let store = gammaboard::PgStore::new(self.harness.pool.clone());
        // Repeated pause must be idempotent, including while the first is draining.
        gammaboard::api::runs::pause_run(&store, self.run).await?;
        gammaboard::api::runs::pause_run(&store, self.run).await?;
        self.harness
            .wait_for(
                "pause drains all roles",
                Duration::from_secs(20),
                || async {
                    Ok(sqlx::query_scalar::<_, i64>(
                        "SELECT count(*) FROM nodes WHERE desired_run_id=$1 OR active_run_id=$1",
                    )
                    .bind(self.run)
                    .fetch_one(&self.harness.pool)
                    .await?
                        == 0)
                },
            )
            .await?;
        self.check().await?;
        let saved = self
            .harness
            .run_sampler_checkpoint(self.run)
            .await?
            .expect("paused checkpoint");
        self.event(json!({"paused_at":saved["completed_samples"], "generation":saved["runtime_state"]["generation"]}))?;
        Ok(saved)
    }

    async fn check(&mut self) -> anyhow::Result<()> {
        // Read one consistent database image: concurrent flushes are legitimate.
        let mut tx = self.harness.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let (state, failure, produced, completed): (String, Option<String>, i64, i64) = sqlx::query_as(
            "SELECT state,failure_reason,nr_produced_samples,nr_completed_samples FROM run_tasks WHERE id=$1")
            .bind(self.task).fetch_one(&mut *tx).await?;
        anyhow::ensure!(
            state != "failed",
            "unexpected terminal failure: {failure:?}"
        );
        anyhow::ensure!(
            0 <= completed && completed <= produced && produced <= SAMPLES,
            "invalid task accounting: {completed} <= {produced} <= {SAMPLES}"
        );
        let observable: Option<JsonValue> =
            sqlx::query_scalar("SELECT current_observable FROM runs WHERE id=$1")
                .bind(self.run)
                .fetch_one(&mut *tx)
                .await?;
        if let Some(value) = observable {
            check_prefix(&value, self.offset)?;
        }
        let value: JsonValue = sqlx::query_scalar(
            "SELECT sampler_checkpoint FROM run_sampler_checkpoints WHERE run_id=$1",
        )
        .bind(self.run)
        .fetch_one(&mut *tx)
        .await?;
        let checkpoint: SamplerAggregatorCheckpoint = serde_json::from_value(value.clone())?;
        anyhow::ensure!(
            checkpoint.task_id == self.task,
            "checkpoint changed task identity"
        );
        let saved_count = check_prefix(&checkpoint.observable_state.to_json()?, self.offset)?;
        anyhow::ensure!(
            saved_count as i64 == checkpoint.completed_samples,
            "checkpoint accumulator/cursor disagree"
        );
        let retained: i64 = sqlx::query_scalar("SELECT COALESCE(sum(batch_size),0)::bigint FROM batches WHERE run_id=$1 AND task_id=$2 AND id>$3 AND id<=$4")
            .bind(self.run).bind(self.task).bind(value["queue"]["last_completed_batch_id"].as_i64().unwrap_or(0))
            .bind(value["queue"]["last_produced_batch_id"].as_i64().unwrap_or(0)).fetch_one(&mut *tx).await?;
        let durable_produced = value["runtime_state"]["produced_samples_total"]
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("checkpoint has no produced cursor"))?;
        anyhow::ensure!(
            retained == durable_produced - checkpoint.completed_samples,
            "cleanup removed work needed by the durable checkpoint"
        );
        let owners: i64 = sqlx::query_scalar("SELECT count(*) FROM nodes WHERE active_run_id=$1 AND active_role='sampler_aggregator' AND lease_expires_at>now()")
            .bind(self.run).fetch_one(&mut *tx).await?;
        anyhow::ensure!(owners <= 1, "multiple live sampler owners");
        tx.commit().await?;
        self.checks += 1;
        self.event(json!({"checked":self.checks,"state":state,"produced":produced,"completed":completed,"durable_completed":saved_count}))
    }

    async fn replace(&mut self, node: &str) -> anyhow::Result<()> {
        self.harness.kill_child(node).await?;
        self.harness.expire_node(node).await?;
        self.harness.start_node(node).await
    }

    async fn act(&mut self, action: Action, rng: &mut Xoshiro256StarStar) -> anyhow::Result<()> {
        let before = self.harness.run_sample_progress(self.run).await?.1;
        anyhow::ensure!(
            before + 17 < SAMPLES,
            "workload ended before all faults were exercised"
        );
        self.wait_progress(before + 17).await?;
        self.event(json!({"action":action}))?;
        match action {
            Action::Pause => {
                let saved = self.pause().await?;
                sleep(Duration::from_millis(rng.random_range(1..40))).await;
                anyhow::ensure!(
                    self.harness
                        .run_sampler_checkpoint(self.run)
                        .await?
                        .as_ref()
                        == Some(&saved),
                    "paused checkpoint changed without a resume"
                );
            }
            Action::KillSampler => {
                let saved = self.harness.run_sampler_checkpoint(self.run).await?;
                self.harness.kill_child("s").await?;
                anyhow::ensure!(
                    self.harness.run_sampler_checkpoint(self.run).await? == saved,
                    "SIGKILL unexpectedly advanced the durable checkpoint"
                );
                self.harness.expire_node("s").await?;
                self.harness.start_node("s").await?;
            }
            Action::KillEvaluator => {
                self.replace(if rng.random() { "e0" } else { "e1" }).await?;
            }
            Action::RestartFleet => {
                // No orderly checkpoint: all new processes recover using the existing DB.
                self.harness.stop_children().await;
                for node in ["s", "e0", "e1"] {
                    self.harness.expire_node(node).await?;
                }
                self.harness.start_nodes(&["s", "e0", "e1"]).await?;
            }
            Action::DisconnectDatabase => {
                sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid()")
                    .execute(&self.harness.pool).await?;
                // This fault is scoped to this test database, never another deployment.
                sleep(Duration::from_millis(150)).await;
            }
            Action::Tune => {
                let batch = rng.random_range(17..62);
                let generation = rng.random_range(97..1537);
                self.event(json!({"batch":batch,"generation":generation}))?;
                gammaboard::api::runs::update_task_queue_tuning(
                    &gammaboard::PgStore::new(self.harness.pool.clone()),
                    self.run,
                    self.task,
                    Some(SamplerQueueTuning {
                        fixed_batch_size: Some(batch),
                        max_generation_size: std::num::NonZeroUsize::new(generation),
                        ..Default::default()
                    }),
                )
                .await?;
            }
            Action::StaleEvaluator => {
                self.harness.wait_for("e0 owns work before suspension", Duration::from_secs(10), || async {
                    Ok(sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM batches WHERE run_id=$1 AND status='claimed' AND claimed_by_node_name='e0')")
                        .bind(self.run).fetch_one(&self.harness.pool).await?)
                }).await?;
                self.harness.signal_child("e0", libc::SIGSTOP)?;
                self.harness.expire_node("e0").await?;
                self.harness.wait_for("expired worker claims are reclaimed", Duration::from_secs(15), || async {
                    Ok(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM batches WHERE run_id=$1 AND status='claimed' AND claimed_by_node_name='e0'")
                        .bind(self.run).fetch_one(&self.harness.pool).await? == 0)
                }).await?;
                self.harness.signal_child("e0", libc::SIGCONT)?;
                sleep(Duration::from_millis(50)).await;
                self.replace("e0").await?;
            }
            Action::AbortCheckpoint => {
                let saved = self.harness.run_sampler_checkpoint(self.run).await?;
                let stages = self.harness.run_stage_snapshot_count(self.run).await?;
                let mut lock = self.harness.pool.begin().await?;
                // Block only publication, allowing maintenance readers to finish.
                sqlx::query(
                    "SELECT run_id FROM run_sampler_checkpoints WHERE run_id=$1 FOR UPDATE",
                )
                .bind(self.run)
                .execute(&mut *lock)
                .await?;
                gammaboard::api::runs::pause_run(
                    &gammaboard::PgStore::new(self.harness.pool.clone()),
                    self.run,
                )
                .await?;
                self.harness.wait_for("checkpoint writer blocked before commit",Duration::from_secs(15),|| async {
                    Ok(sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE '%INSERT INTO run_sampler_checkpoints%')")
                        .fetch_one(&self.harness.pool).await?)
                }).await?;
                self.harness.kill_child("s").await?;
                // Abort the blocked server transaction too: a client dying alone need not cancel it.
                sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE '%INSERT INTO run_sampler_checkpoints%'")
                    .execute(&self.harness.pool).await?;
                lock.rollback().await?;
                anyhow::ensure!(
                    self.harness.run_sampler_checkpoint(self.run).await? == saved,
                    "aborted checkpoint became visible"
                );
                anyhow::ensure!(
                    self.harness.run_stage_snapshot_count(self.run).await? == stages,
                    "aborted checkpoint published a partial stage"
                );
                self.harness.expire_node("s").await?;
                self.harness.start_node("s").await?;
            }
        }
        self.resume().await?;
        sleep(Duration::from_millis(rng.random_range(15..90))).await;
        self.check().await
    }

    async fn exercise(&mut self, seed: u64) -> anyhow::Result<()> {
        let mut rng = Xoshiro256StarStar::seed_from_u64(seed);
        // Establish an actual partial-generation checkpoint before randomized interruption.
        let saved = self.pause().await?;
        anyhow::ensure!(
            saved["runtime_state"]["generation"]["pending"].is_object(),
            "fixture failed to reach an undispatched generation boundary"
        );
        if self.training {
            anyhow::ensure!(
                saved["runtime_state"]["generation"]["training_groups"][0]["values"]
                    .as_array()
                    .is_some_and(|values| !values.is_empty()),
                "fixture did not checkpoint partial training feedback"
            );
        }
        self.resume().await?;
        self.wait_progress(saved["completed_samples"].as_i64().unwrap() + 7)
            .await?;
        let mut actions = vec![
            Action::Pause,
            Action::KillSampler,
            Action::KillEvaluator,
            Action::RestartFleet,
            Action::DisconnectDatabase,
            Action::Tune,
            Action::StaleEvaluator,
            Action::AbortCheckpoint,
        ];
        actions.shuffle(&mut rng);
        for _ in 0..rng.random_range(1..=4) {
            actions.push(
                [
                    Action::Pause,
                    Action::KillEvaluator,
                    Action::Tune,
                    Action::KillSampler,
                ][rng.random_range(0..4)],
            );
        }
        for action in actions {
            self.act(action, &mut rng).await?;
        }
        self.event(json!({"faults_stopped":true}))?;
        self.wait_progress(SAMPLES).await?;
        wait_for_task_state(
            &self.harness,
            self.run,
            "completed",
            Duration::from_secs(30),
        )
        .await?;
        self.check().await?;
        let saved = self
            .harness
            .run_sampler_checkpoint(self.run)
            .await?
            .unwrap();
        let checkpoint: SamplerAggregatorCheckpoint = serde_json::from_value(saved.clone())?;
        anyhow::ensure!(
            check_prefix(&checkpoint.observable_state.to_json()?, self.offset)? == SAMPLES as usize,
            "final exact sequence is incomplete"
        );
        let sampler = &saved["sampler_snapshot"]["raw"]["sampler_state"];
        anyhow::ensure!(
            sampler["produced"] == SAMPLES,
            "sampler lost its generation cursor"
        );
        anyhow::ensure!(
            sampler["accepted"] == if self.training { SAMPLES } else { 0 },
            "feedback missing or delivered during inference"
        );
        anyhow::ensure!(
            sampler["pending"].as_array().is_some_and(Vec::is_empty),
            "undelivered feedback after completion"
        );
        let open: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM batches WHERE run_id=$1 AND status IN ('pending','claimed')",
        )
        .bind(self.run)
        .fetch_one(&self.harness.pool)
        .await?;
        anyhow::ensure!(open == 0, "orphan work after completion");
        self.event(json!({"passed":true,"seed":seed,"training":self.training,"samples":SAMPLES,"invariant_checks":self.checks}))
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires PostgreSQL and Python with NumPy; seeded real-process recovery test"]
async fn recovery_state_machine() -> anyhow::Result<()> {
    let seeds = std::env::var("GAMMABOARD_RECOVERY_SEEDS").unwrap_or_else(|_| "17,41".into());
    let mut cases = Vec::new();
    for seed in seeds.split(',').map(str::parse::<u64>) {
        let seed = seed?;
        for training in [false, true] {
            let started = Instant::now();
            let mut test = Recovery::new(seed, training).await?;
            test.event(json!({"seed":seed,"training":training,"samples":SAMPLES,"started_at":chrono::Utc::now()}))?;
            let result = tokio::time::timeout(Duration::from_secs(180), test.exercise(seed))
                .await
                .unwrap_or_else(|_| {
                    Err(anyhow::anyhow!("recovery case exceeded its 180s deadline"))
                });
            if let Err(error) = &result {
                let rows: JsonValue = tokio::time::timeout(Duration::from_secs(5),sqlx::query_scalar("SELECT jsonb_build_object('nodes',(SELECT jsonb_agg(to_jsonb(n)) FROM nodes n),'logs',(SELECT jsonb_agg(to_jsonb(l)) FROM runtime_logs l),'runs',(SELECT jsonb_agg(to_jsonb(r)) FROM runs r),'tasks',(SELECT jsonb_agg(to_jsonb(t)) FROM run_tasks t),'checkpoints',(SELECT jsonb_agg(to_jsonb(c)) FROM run_sampler_checkpoints c),'batches',(SELECT jsonb_agg(to_jsonb(b)) FROM batches b))")
                    .fetch_one(&test.harness.pool)).await.ok().and_then(Result::ok).unwrap_or(json!(null));
                std::fs::write(
                    test.harness.artifacts.join("failure.json"),
                    serde_json::to_vec_pretty(
                        &json!({"error":format!("{error:#}"),"database":rows}),
                    )?,
                )?;
                eprintln!(
                    "Recovery seed {seed}, training={training}: {error:#}; evidence: {}",
                    test.harness.artifacts.display()
                );
            }
            cases.push(
                json!({"seed":seed,"training":training,"passed":result.is_ok(),
                "seconds":started.elapsed().as_secs_f64(),"samples":SAMPLES,
                "invariant_checks":test.checks,"artifacts":test.harness.artifacts,
                "actions":test.trace.iter().filter_map(|e|e.get("action")).collect::<Vec<_>>()}),
            );
            std::fs::write(
                test.harness
                    .artifacts
                    .parent()
                    .unwrap()
                    .join("recovery-summary.json"),
                serde_json::to_vec_pretty(&json!({"cases":cases}))?,
            )?;
            let cleanup = test.harness.cleanup().await;
            result?;
            cleanup?;
        }
    }
    Ok(())
}
