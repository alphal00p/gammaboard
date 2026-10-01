use gammaboard::core::{
    AccumulatorMetricName, BatchFailOutcome, ControlPlaneStore, RunReadStore, RunTaskInput,
    RunTaskSpec, RunTaskStore, SampleStopCondition, StoreError, TaskMeasurementOutput,
    WorkQueueStore, WorkerRole, next_batch_ids,
};
use gammaboard::{Batch, LatentBatchSpec, MeasurementResult, PgStore, Point};
use sqlx::postgres::PgPoolOptions;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tokio::time::{Duration, sleep};

static TEST_LOCK: Mutex<()> = Mutex::const_new(());

#[tokio::test]
#[ignore = "requires postgres"]
async fn batch_notifications_survive_cancelled_waits_and_use_the_existing_pool() {
    let _guard = TEST_LOCK.lock().await;
    let url = std::env::var("GAMMABOARD_TEST_DATABASE_URL").unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let run_id = -rand::random_range(1..i32::MAX);
    let store = PgStore::new(pool.clone())
        .listen_for_batches(run_id)
        .await
        .unwrap();
    // Subscription precedes the empty claim. An arrival between that query
    // and waiting must remain visible, and sending must fit in this same pool.
    assert!(
        store
            .claim_batch(run_id, "missing-node", "missing-token")
            .await
            .unwrap()
            .is_none()
    );
    store.notify_work_available(run_id).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), store.wait_for_work(run_id))
        .await
        .expect("hint must not be lost between claim and wait")
        .unwrap();
    assert_eq!(pool.size(), 2);

    // Cancelling a wait leaves the listener usable, and another run cannot
    // wake it. Unlike the runner's safety wait, this store wait has no timer.
    store.notify_work_available(run_id - 1).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), store.wait_for_work(run_id))
            .await
            .is_err()
    );
    store.notify_work_available(run_id).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), store.wait_for_work(run_id))
        .await
        .expect("cancelled wait must not lose the subscription")
        .unwrap();
    drop(store);
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn copy_inputs_round_trip_mixed_payloads_and_row_lengths() {
    use gammaboard::core::AccumulatorConfig;
    use gammaboard::utils::rng::SerializableMonteCarloRng;
    use gammaboard::{LatentBatch, LatentBatchPayload};

    let (_test_guard, store) = locked_test_store().await;
    let (run, task, _, _) = reliability_fixture(&store, 0).await;
    let batches = [
        LatentBatchSpec::from_batch(
            &Batch::from_points([
                Point::new(vec![0.25], vec![], 2.0),
                Point::new(vec![0.5, 0.75, 0.125], vec![7, 2], 3.0),
            ])
            .unwrap(),
        )
        .build(),
        LatentBatch {
            nr_samples: 777,
            accumulator: AccumulatorConfig::scalar(),
            payload: LatentBatchPayload::HavanaInference {
                rng_state: SerializableMonteCarloRng::new(42, 0),
            },
        },
        LatentBatchSpec::from_batch(
            &Batch::from_points(
                (0..1024).map(|i| Point::new(vec![i as f64 / 1024.0; 6], vec![], 1.0)),
            )
            .unwrap(),
        )
        .build(),
    ];
    let ids = next_batch_ids(batches.len());
    let outcome = store
        .insert_batches(run, task, false, &ids, &batches)
        .await
        .unwrap();
    let rows: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT batch_id, latent_batch FROM batch_inputs WHERE batch_id = ANY($1) ORDER BY batch_id"
    ).bind(&ids).fetch_all(store.pool()).await.unwrap();
    assert_eq!(rows.len(), batches.len());
    assert_eq!(
        outcome.metrics.payload_bytes,
        rows.iter().map(|(_, bytes)| bytes.len()).sum::<usize>()
    );
    for ((id, bytes), (expected_id, batch)) in rows.iter().zip(ids.iter().zip(&batches)) {
        assert_eq!(id, expected_id);
        assert_eq!(*bytes, batch.to_bytes().unwrap());
        assert_eq!(LatentBatch::from_bytes(bytes).unwrap(), *batch);
    }
    store.remove_run(run).await.unwrap();
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn input_transaction_does_not_block_other_batches_and_counters_commit_atomically() {
    let (_test_guard, store) = locked_test_store().await;
    let run: i32 = sqlx::query_scalar(
        "INSERT INTO runs (name, integration_params, point_spec) VALUES
         ('deferred-counters', '{}', '{\"continuous\":{\"dims\":1}}') RETURNING id",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    let task = insert_completed_pause_task(&store, run).await;
    let node = unique_id("counter-node");
    store
        .announce_node(&node, &node, &Default::default())
        .await
        .unwrap();
    store
        .set_current_assignment(&node, WorkerRole::Evaluator, run)
        .await
        .unwrap();
    let batch = LatentBatchSpec::from_batch(
        &Batch::from_points([Point::new(vec![0.5], Vec::new(), 1.0)]).unwrap(),
    )
    .build();
    store
        .insert_batches(
            run,
            task,
            false,
            &next_batch_ids(1),
            std::slice::from_ref(&batch),
        )
        .await
        .unwrap();

    let mut writer = store.pool().begin().await.unwrap();
    sqlx::query("INSERT INTO batches (run_id, task_id, batch_size) VALUES ($1,$2,1)")
        .bind(run)
        .bind(task)
        .execute(&mut *writer)
        .await
        .unwrap();
    // Keep the input transaction open at the point where COPY would run. An
    // unrelated committed batch must still be claimable and completable.
    tokio::time::timeout(Duration::from_secs(2), async {
        let token = unique_id("counter-claim");
        let claim = store
            .claim_batch(run, &node, &token)
            .await
            .unwrap()
            .unwrap();
        store
            .submit_batch_results(claim.batch_id, &node, &token, &empty_batch_result(), 1.0)
            .await
            .unwrap();
    })
    .await
    .expect("payload writer must not hold the shared queue counter lock");
    writer.rollback().await.unwrap();
    let counters_sql = "SELECT total_batches,pending_batches,claimed_batches,completed_batches
                        FROM run_batch_queue_counters WHERE run_id=$1";
    let counters: (i64, i64, i64, i64) = sqlx::query_as(counters_sql)
        .bind(run)
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(
        counters,
        (1, 0, 0, 1),
        "rollback must discard the pending counter change"
    );

    let mut writer = store.pool().begin().await.unwrap();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO batches (run_id, task_id, batch_size) VALUES ($1,$2,1) RETURNING id",
    )
    .bind(run)
    .bind(task)
    .fetch_one(&mut *writer)
    .await
    .unwrap();
    sqlx::query("INSERT INTO batch_inputs (batch_id,latent_batch) VALUES ($1,$2)")
        .bind(id)
        .bind(batch.to_bytes().unwrap())
        .execute(&mut *writer)
        .await
        .unwrap();
    let before: (i64, i64, i64, i64) = sqlx::query_as(counters_sql)
        .bind(run)
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(
        before, counters,
        "uncommitted inputs must not affect visible counters"
    );
    writer.commit().await.unwrap();
    let after: (i64, i64, i64, i64) = sqlx::query_as(counters_sql)
        .bind(run)
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(after, (2, 1, 0, 1));
    store.remove_run(run).await.unwrap();
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_batch_queue_counters WHERE run_id=$1")
            .bind(run)
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(
        remaining, 0,
        "deferred cleanup must not recreate a deleted run's counters"
    );
}

fn unique_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_nanos();
    format!("{prefix}-{nanos}")
}

async fn locked_test_store() -> (tokio::sync::MutexGuard<'static, ()>, PgStore) {
    let guard = TEST_LOCK.lock().await;
    let db_url = std::env::var("GAMMABOARD_TEST_DATABASE_URL")
        .expect("set GAMMABOARD_TEST_DATABASE_URL to an isolated, migrated test database");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url)
        .await
        .expect("connect to explicitly configured test database");
    // These tests share one isolated database under TEST_LOCK. A removed run
    // leaves its workers alive, so their leases must not affect the next test.
    sqlx::raw_sql("DELETE FROM nodes; DELETE FROM node_launch_requests;")
        .execute(&pool)
        .await
        .expect("clear worker fixtures from preceding tests");
    (guard, PgStore::new(pool))
}

async fn insert_completed_pause_task(store: &PgStore, run_id: i32) -> i64 {
    sqlx::query_scalar(
        r#"
        INSERT INTO run_tasks (run_id, name, sequence_nr, task, task_toml, state)
        VALUES ($1, 'sample-0', 0, '{"kind":"pause"}'::jsonb, $2, 'completed')
        RETURNING id
        "#,
    )
    .bind(run_id)
    .bind("kind = \"pause\"\n")
    .fetch_one(store.pool())
    .await
    .expect("insert run task")
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn active_task_accumulates_declared_cpu_time() {
    let (_test_guard, store) = locked_test_store().await;
    let node_name = unique_id("cpu-node");
    let node_uuid = unique_id("cpu-uuid");
    let run_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (name, integration_params, point_spec)
        VALUES ('cpu-time', '{}'::jsonb, '{"continuous":{"dims":1}}'::jsonb)
        RETURNING id
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("insert run");
    let task = store
        .append_run_tasks(
            run_id,
            &[RunTaskInput {
                name: Some("sample".to_string()),
                task: RunTaskSpec::Sample {
                    publish_result: true,
                    stop_condition: SampleStopCondition {
                        max_samples: Some(1),
                        ..Default::default()
                    },
                    measurement: None,
                    evaluator: None,
                    sampler_aggregator: None,
                    accumulator: None,
                    queue_tuning: None,
                    batch_transforms: None,
                },
            }],
        )
        .await
        .expect("append task")
        .remove(0);
    store
        .activate_next_run_task(run_id)
        .await
        .expect("activate task");

    let capabilities = [("cpus".to_string(), 2)].into_iter().collect();
    store
        .announce_node(&node_name, &node_uuid, &capabilities)
        .await
        .expect("announce node");
    store
        .set_current_assignment(&node_uuid, WorkerRole::Evaluator, run_id)
        .await
        .expect("assign node");
    store
        .announce_node(&node_name, &node_uuid, &capabilities)
        .await
        .unwrap();
    // A progress writer may hold the task row while pause/activity updates
    // proceed. Those node updates must not acquire the CPU-accounting lock.
    let mut task_writer = store.pool().begin().await.expect("task writer");
    sqlx::query("SELECT id FROM run_tasks WHERE id = $1 FOR UPDATE")
        .bind(task.id)
        .fetch_one(&mut *task_writer)
        .await
        .expect("lock task");
    tokio::time::timeout(Duration::from_secs(2), async {
        sqlx::query("UPDATE nodes SET activity = '{\"phase\":\"waiting\"}', desired_run_id = NULL, desired_role = NULL WHERE uuid = $1")
            .bind(&node_uuid)
            .execute(store.pool())
            .await
            .expect("activity and pause do not write task CPU time");
        store.announce_node(&node_name, &node_uuid, &capabilities).await
            .expect("renewal updates its own accounting row while task progress is locked");
    })
    .await
    .expect("node metadata must not wait for the task lock");
    task_writer.rollback().await.expect("release task writer");
    sleep(Duration::from_millis(50)).await;
    store
        .announce_node(&node_name, &node_uuid, &capabilities)
        .await
        .expect("account heartbeat");
    let before_completion = store
        .load_run_task(task.id)
        .await
        .unwrap()
        .unwrap()
        .cpu_seconds;
    sleep(Duration::from_millis(30)).await;
    store
        .complete_run_task(task.id)
        .await
        .expect("complete task");

    let completed = store
        .load_run_task(task.id)
        .await
        .expect("load task")
        .expect("task exists");
    assert!(
        completed.cpu_seconds >= 0.08,
        "two allocated CPUs should accumulate about twice the elapsed wall time: {}",
        completed.cpu_seconds
    );
    let accounted = completed.cpu_seconds;
    assert!(
        accounted >= before_completion + 0.04,
        "completion must flush the final accounting interval"
    );
    sleep(Duration::from_millis(20)).await;
    store
        .announce_node(&node_name, &node_uuid, &capabilities)
        .await
        .expect("post-completion heartbeat");
    let unchanged = store
        .load_run_task(task.id)
        .await
        .expect("load completed task")
        .expect("task exists");
    assert_eq!(unchanged.cpu_seconds, accounted);
    let run = store
        .get_run_progress(run_id)
        .await
        .expect("load run")
        .expect("run exists");
    assert_eq!(run.cpu_seconds, accounted);
    assert_eq!(run.cpu_seconds_including_children, accounted);

    store.remove_run(run_id).await.expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn parent_task_and_run_include_child_cpu_time() {
    let (_test_guard, store) = locked_test_store().await;
    let parent_run_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (name, integration_params, point_spec)
        VALUES ('cpu-parent', '{}'::jsonb, '{"continuous":{"dims":1}}'::jsonb)
        RETURNING id
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("insert parent run");
    let parent_task_id = store
        .append_run_tasks(
            parent_run_id,
            &[RunTaskInput {
                name: Some("controller".to_string()),
                task: RunTaskSpec::SetAccumulator {
                    accumulator: gammaboard::core::AccumulatorConfig::Empty,
                },
            }],
        )
        .await
        .expect("append parent task")[0]
        .id;
    sqlx::query("UPDATE run_tasks SET state = 'completed', cpu_seconds = 60.0 WHERE id = $1")
        .bind(parent_task_id)
        .execute(store.pool())
        .await
        .expect("complete parent task");
    let child_run_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (
            name, integration_params, point_spec, parent_run_id, parent_task_id
        ) VALUES (
            'cpu-child', '{}'::jsonb, '{"continuous":{"dims":1}}'::jsonb, $1, $2
        )
        RETURNING id
        "#,
    )
    .bind(parent_run_id)
    .bind(parent_task_id)
    .fetch_one(store.pool())
    .await
    .expect("insert child run");
    let child_task_id = store
        .append_run_tasks(
            child_run_id,
            &[RunTaskInput {
                name: Some("child-task".to_string()),
                task: RunTaskSpec::SetAccumulator {
                    accumulator: gammaboard::core::AccumulatorConfig::Empty,
                },
            }],
        )
        .await
        .expect("append child task")[0]
        .id;
    sqlx::query("UPDATE run_tasks SET state = 'completed', cpu_seconds = 120.0 WHERE id = $1")
        .bind(child_task_id)
        .execute(store.pool())
        .await
        .expect("complete child task");

    let parent_task = store
        .list_run_tasks(parent_run_id)
        .await
        .expect("list parent tasks")
        .remove(0);
    assert_eq!(parent_task.cpu_seconds, 60.0);
    assert_eq!(parent_task.cpu_seconds_including_children, 180.0);
    let parent_run = store
        .get_run_progress(parent_run_id)
        .await
        .expect("load parent run")
        .expect("parent run exists");
    assert_eq!(parent_run.cpu_seconds, 60.0);
    assert_eq!(parent_run.cpu_seconds_including_children, 180.0);

    store.remove_run(parent_run_id).await.expect("cleanup runs");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn claim_batch_requires_active_assignment() {
    let (_test_guard, store) = locked_test_store().await;
    let node_name = unique_id("node");
    let node_uuid = unique_id("uuid");

    let run_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (
            name,
            integration_params,
            point_spec
        ) VALUES (
            'claim-batch-active',
            '{}'::jsonb,
            '{"continuous":{"dims":1}}'::jsonb
        )
        RETURNING id
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("insert run");

    store
        .announce_node(&node_name, &node_uuid, &Default::default())
        .await
        .expect("announce node");
    store
        .set_current_assignment(&node_uuid, WorkerRole::Evaluator, run_id)
        .await
        .expect("set current evaluator assignment");

    let task_id = insert_completed_pause_task(&store, run_id).await;

    let batch = Batch::from_points([Point::new(vec![1.0], Vec::new(), 1.0)]).expect("batch");
    let latent_batch = LatentBatchSpec::from_batch(&batch).build();
    let batch_ids = next_batch_ids(1);
    store
        .insert_batches(
            run_id,
            task_id,
            false,
            &batch_ids,
            std::slice::from_ref(&latent_batch),
        )
        .await
        .expect("insert batch");

    let claimed = store
        .claim_batch(run_id, &node_uuid, &unique_id("claim"))
        .await
        .expect("claim batch");
    assert!(
        claimed.is_some(),
        "assigned evaluator should be able to claim"
    );

    store.remove_run(run_id).await.expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn task_measurement_output_round_trips() {
    let (_test_guard, store) = locked_test_store().await;

    let run_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (
            name,
            integration_params,
            point_spec
        ) VALUES (
            'task-measurement-output',
            '{}'::jsonb,
            '{"continuous":{"dims":1}}'::jsonb
        )
        RETURNING id
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("insert run");

    let tasks = store
        .append_run_tasks(
            run_id,
            &[RunTaskInput {
                name: Some("sample".to_string()),
                task: RunTaskSpec::Sample {
                    publish_result: true,
                    stop_condition: SampleStopCondition {
                        max_samples: Some(10),
                        ..SampleStopCondition::default()
                    },
                    measurement: None,
                    evaluator: None,
                    sampler_aggregator: None,
                    accumulator: None,
                    queue_tuning: None,
                    batch_transforms: None,
                },
            }],
        )
        .await
        .expect("append task");
    let task_id = tasks[0].id;

    store
        .persist_task_measurement_output(
            task_id,
            &TaskMeasurementOutput::Completed {
                results: vec![MeasurementResult {
                    name: AccumulatorMetricName::Mean,
                    component: None,
                    value: 1.25,
                    uncertainty: Some(0.1),
                    sample_count: 10,
                }],
            },
        )
        .await
        .expect("persist measurement output");

    let task = store
        .load_run_task(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    let Some(TaskMeasurementOutput::Completed { results }) = task.measurement_output else {
        panic!("missing completed measurement output");
    };
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, AccumulatorMetricName::Mean);
    assert_eq!(results[0].value, 1.25);

    store.remove_run(run_id).await.expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn claim_batch_rejects_unassigned_or_inactive_assignment() {
    let (_test_guard, store) = locked_test_store().await;
    let node_name = unique_id("node");
    let node_uuid = unique_id("uuid");

    let run_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (
            name,
            integration_params,
            point_spec
        ) VALUES (
            'claim-batch-inactive',
            '{}'::jsonb,
            '{"continuous":{"dims":1}}'::jsonb
        )
        RETURNING id
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("insert run");

    store
        .announce_node(&node_name, &node_uuid, &Default::default())
        .await
        .expect("announce node");

    let task_id = insert_completed_pause_task(&store, run_id).await;

    let batch = Batch::from_points([Point::new(vec![2.0], Vec::new(), 1.0)]).expect("batch");
    let latent_batch = LatentBatchSpec::from_batch(&batch).build();
    let batch_ids = next_batch_ids(1);
    store
        .insert_batches(
            run_id,
            task_id,
            false,
            &batch_ids,
            std::slice::from_ref(&latent_batch),
        )
        .await
        .expect("insert batch");

    let unassigned_claim = store
        .claim_batch(run_id, &node_uuid, &unique_id("claim"))
        .await
        .expect("claim batch while unassigned");
    assert!(
        unassigned_claim.is_none(),
        "unassigned evaluator should not be able to claim"
    );

    store
        .set_current_assignment(&node_uuid, WorkerRole::Evaluator, run_id)
        .await
        .expect("set current evaluator assignment");
    store
        .clear_current_assignment(&node_uuid)
        .await
        .expect("clear current assignment");

    let inactive_claim = store
        .claim_batch(run_id, &node_uuid, &unique_id("claim"))
        .await
        .expect("claim batch while inactive");
    assert!(
        inactive_claim.is_none(),
        "inactive assignment should not be able to claim"
    );

    store.remove_run(run_id).await.expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn claim_batch_claims_exactly_one_pending_batch() {
    let (_test_guard, store) = locked_test_store().await;
    let node_name = unique_id("node");
    let node_uuid = unique_id("uuid");

    let run_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (
            name,
            integration_params,
            point_spec
        ) VALUES (
            'claim-batch-single-row',
            '{}'::jsonb,
            '{"continuous":{"dims":1}}'::jsonb
        )
        RETURNING id
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("insert run");

    store
        .announce_node(&node_name, &node_uuid, &Default::default())
        .await
        .expect("announce node");
    store
        .set_current_assignment(&node_uuid, WorkerRole::Evaluator, run_id)
        .await
        .expect("set current evaluator assignment");

    let task_id = insert_completed_pause_task(&store, run_id).await;

    let batch = Batch::from_points([Point::new(vec![3.0], Vec::new(), 1.0)]).expect("batch");
    let latent_batch = LatentBatchSpec::from_batch(&batch).build();
    let batches = vec![
        latent_batch.clone(),
        latent_batch.clone(),
        latent_batch.clone(),
        latent_batch,
    ];
    let batch_ids = next_batch_ids(batches.len());
    store
        .insert_batches(run_id, task_id, false, &batch_ids, &batches)
        .await
        .expect("insert batches");

    let claimed = store
        .claim_batch(run_id, &node_uuid, &unique_id("claim"))
        .await
        .expect("claim batch");
    assert!(
        claimed.is_some(),
        "assigned evaluator should claim one batch"
    );

    let claimed_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM batches
        WHERE run_id = $1
          AND status = 'claimed'
          AND claimed_by_node_uuid = $2
        "#,
    )
    .bind(run_id)
    .bind(&node_uuid)
    .fetch_one(store.pool())
    .await
    .expect("count claimed batches");
    assert_eq!(
        claimed_count, 1,
        "claim_batch should claim exactly one pending batch per call"
    );

    let pending_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM batches
        WHERE run_id = $1
          AND status = 'pending'
        "#,
    )
    .bind(run_id)
    .fetch_one(store.pool())
    .await
    .expect("count pending batches");
    assert_eq!(pending_count, 3, "remaining batches should stay pending");

    store.remove_run(run_id).await.expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn sampler_aggregator_desired_assignment_is_unique_per_run() {
    let (_test_guard, store) = locked_test_store().await;
    let node_a = unique_id("node-a");
    let node_b = unique_id("node-b");
    let node_a_uuid = unique_id("uuid-a");
    let node_b_uuid = unique_id("uuid-b");

    let run_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (
            name,
            integration_params,
            point_spec
        ) VALUES (
            'test-run',
            '{}'::jsonb,
            '{"continuous":{"dims":0}}'::jsonb
        )
        RETURNING id
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("insert run");

    store
        .announce_node(&node_a, &node_a_uuid, &Default::default())
        .await
        .expect("announce first node");
    store
        .announce_node(&node_b, &node_b_uuid, &Default::default())
        .await
        .expect("announce second node");

    store
        .assign_worker_pool(&node_a, WorkerRole::SamplerAggregator, run_id)
        .await
        .expect("assign first sampler");

    let err = store
        .assign_worker_pool(&node_b, WorkerRole::SamplerAggregator, run_id)
        .await
        .expect_err("second sampler assignment should fail");

    match err {
        StoreError::InvalidInput(message) => {
            assert!(
                message.contains("sampler_aggregator assignment"),
                "unexpected error message: {message}"
            );
        }
        other => panic!("expected invalid input, got {other}"),
    }

    store.remove_run(run_id).await.expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn cleanup_consumed_completed_batches_does_not_remove_failed_batches() {
    let (_test_guard, store) = locked_test_store().await;

    let run_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (
            name,
            integration_params,
            point_spec
        ) VALUES (
            'cleanup-failed-batches',
            '{}'::jsonb,
            '{"continuous":{"dims":1}}'::jsonb
        )
        RETURNING id
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("insert run");

    let task_id = insert_completed_pause_task(&store, run_id).await;

    let batch = Batch::from_points([Point::new(vec![1.0], Vec::new(), 1.0)]).expect("batch");
    let latent_batch = LatentBatchSpec::from_batch(&batch).build();
    let batch_ids = next_batch_ids(1);
    store
        .insert_batches(run_id, task_id, false, &batch_ids, &[latent_batch])
        .await
        .expect("insert batch");

    let node = unique_id("failure-worker");
    store
        .announce_node(&node, &node, &Default::default())
        .await
        .unwrap();
    store
        .set_current_assignment(&node, WorkerRole::Evaluator, run_id)
        .await
        .unwrap();
    let claim = store
        .claim_batch(run_id, &node, &unique_id("claim"))
        .await
        .unwrap()
        .unwrap();
    let outcome = store
        .fail_batch(batch_ids[0], &node, &claim.claim_token, "forced failure", 1)
        .await
        .expect("fail batch");
    assert!(
        matches!(
            outcome,
            BatchFailOutcome::PermanentlyFailed { retry_count: 1, .. }
        ),
        "batch should become permanently failed at retry limit"
    );

    let removed = store
        .cleanup_consumed_completed_batches(run_id, i64::MAX, 1024)
        .await
        .expect("cleanup completed batches");
    assert_eq!(removed, 0, "cleanup should only remove completed batches");

    let failed_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM batches
        WHERE run_id = $1
          AND status = 'failed'
        "#,
    )
    .bind(run_id)
    .fetch_one(store.pool())
    .await
    .expect("count failed batches");
    assert_eq!(failed_count, 1, "failed batch must remain persisted");

    store.remove_run(run_id).await.expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn expired_sampler_assignment_does_not_block_new_sampler_assignment() {
    let (_test_guard, store) = locked_test_store().await;
    let stale_node = unique_id("stale-sampler");
    let stale_uuid = unique_id("stale-sampler-uuid");
    let fresh_node = unique_id("fresh-sampler");
    let fresh_uuid = unique_id("fresh-sampler-uuid");

    let run_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (
            name,
            integration_params,
            point_spec
        ) VALUES (
            'stale-sampler-assignment-run',
            '{}'::jsonb,
            '{"continuous_dims":0,"discrete_dims":0}'::jsonb
        )
        RETURNING id
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("insert run");

    store
        .announce_node(&stale_node, &stale_uuid, &Default::default())
        .await
        .expect("announce stale node");
    store
        .assign_worker_pool(&stale_node, WorkerRole::SamplerAggregator, run_id)
        .await
        .expect("assign stale sampler");

    // Simulate an ungraceful control/database shutdown: the row's lease expires
    // naturally, but the node process never gets to call expire_node_lease().
    sqlx::query(
        r#"
        UPDATE nodes
        SET lease_expires_at = now() - interval '1 second'
        WHERE name = $1
        "#,
    )
    .bind(&stale_node)
    .execute(store.pool())
    .await
    .expect("expire stale node without cleanup");

    store
        .announce_node(&fresh_node, &fresh_uuid, &Default::default())
        .await
        .expect("announce fresh node");
    store
        .assign_worker_pool(&fresh_node, WorkerRole::SamplerAggregator, run_id)
        .await
        .expect("fresh sampler assignment should reap stale sampler assignment first");

    let stale_assignment = store
        .get_desired_assignment(&stale_node)
        .await
        .expect("load stale desired assignment");
    assert!(
        stale_assignment.is_none(),
        "stale expired sampler assignment should be cleared"
    );

    store.remove_run(run_id).await.expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn assigning_new_role_replaces_existing_desired_assignment_for_node() {
    let (_test_guard, store) = locked_test_store().await;
    let node_name = unique_id("node");
    let node_uuid = unique_id("uuid");

    let run_a: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (
            name,
            integration_params,
            point_spec
        ) VALUES (
            'test-run-a',
            '{}'::jsonb,
            '{"continuous_dims":0,"discrete_dims":0}'::jsonb
        )
        RETURNING id
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("insert run a");

    let run_b: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (
            name,
            integration_params,
            point_spec
        ) VALUES (
            'test-run-b',
            '{}'::jsonb,
            '{"continuous_dims":0,"discrete_dims":0}'::jsonb
        )
        RETURNING id
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("insert run b");

    store
        .announce_node(&node_name, &node_uuid, &Default::default())
        .await
        .expect("announce node");

    store
        .assign_worker_pool(&node_name, WorkerRole::Evaluator, run_a)
        .await
        .expect("assign evaluator");
    store
        .assign_worker_pool(&node_name, WorkerRole::SamplerAggregator, run_b)
        .await
        .expect("replace desired assignment");

    let assignment = store
        .get_desired_assignment(&node_name)
        .await
        .expect("load desired assignment")
        .expect("assignment should exist");
    assert_eq!(assignment.role, WorkerRole::SamplerAggregator);
    assert_eq!(assignment.run_id, run_b);

    let desired_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM nodes
        WHERE name = $1
          AND desired_run_id IS NOT NULL
          AND desired_role IS NOT NULL
        "#,
    )
    .bind(&node_name)
    .fetch_one(store.pool())
    .await
    .expect("count desired assignments");
    assert_eq!(
        desired_count, 1,
        "node should have exactly one desired assignment"
    );

    store
        .clear_desired_assignment(&node_name)
        .await
        .expect("clear desired assignment");
    assert!(
        store
            .get_desired_assignment(&node_name)
            .await
            .expect("load cleared assignment")
            .is_none(),
        "node desired assignment should be cleared"
    );

    store.remove_run(run_a).await.expect("cleanup run a");
    store.remove_run(run_b).await.expect("cleanup run b");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn assigning_dead_node_returns_not_found() {
    let (_test_guard, store) = locked_test_store().await;
    let node_name = unique_id("dead-node");
    let node_uuid = unique_id("dead-uuid");

    let run_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (
            name,
            integration_params,
            point_spec
        ) VALUES (
            'dead-node-run',
            '{}'::jsonb,
            '{"continuous_dims":0,"discrete_dims":0}'::jsonb
        )
        RETURNING id
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("insert run");

    store
        .announce_node(&node_name, &node_uuid, &Default::default())
        .await
        .expect("announce node");
    store
        .expire_node_lease(&node_uuid)
        .await
        .expect("expire node lease");

    let err = store
        .assign_worker_pool(&node_name, WorkerRole::Evaluator, run_id)
        .await
        .expect_err("dead node assignment should fail");

    match err {
        StoreError::NotFound(message) => {
            assert!(
                message.contains("is not live"),
                "unexpected error message: {message}"
            );
        }
        other => panic!("expected not found, got {other}"),
    }

    store.remove_run(run_id).await.expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn expiring_node_lease_clears_desired_assignment() {
    let (_test_guard, store) = locked_test_store().await;
    let node_name = unique_id("expiring-node");
    let node_uuid = unique_id("expiring-uuid");

    let run_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (
            name,
            integration_params,
            point_spec
        ) VALUES (
            'expiring-node-run',
            '{}'::jsonb,
            '{"continuous_dims":0,"discrete_dims":0}'::jsonb
        )
        RETURNING id
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("insert run");

    store
        .announce_node(&node_name, &node_uuid, &Default::default())
        .await
        .expect("announce node");
    store
        .assign_worker_pool(&node_name, WorkerRole::SamplerAggregator, run_id)
        .await
        .expect("assign sampler role");

    store
        .expire_node_lease(&node_uuid)
        .await
        .expect("expire node lease");

    assert!(
        store
            .get_desired_assignment(&node_name)
            .await
            .expect("load desired assignment after expiry")
            .is_none(),
        "desired assignment should be cleared on lease expiry"
    );

    store.remove_run(run_id).await.expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn shutdown_request_clears_desired_assignment_but_keeps_current_assignment() {
    let (_test_guard, store) = locked_test_store().await;
    let node_name = unique_id("shutdown-node");
    let node_uuid = unique_id("shutdown-uuid");

    let run_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (
            name,
            integration_params,
            point_spec
        ) VALUES (
            'shutdown-clears-desired-run',
            '{}'::jsonb,
            '{"continuous_dims":0,"discrete_dims":0}'::jsonb
        )
        RETURNING id
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("insert run");

    store
        .announce_node(&node_name, &node_uuid, &Default::default())
        .await
        .expect("announce node");
    store
        .assign_worker_pool(&node_name, WorkerRole::SamplerAggregator, run_id)
        .await
        .expect("assign desired sampler role");
    store
        .set_current_assignment(&node_uuid, WorkerRole::SamplerAggregator, run_id)
        .await
        .expect("set current sampler role");

    store
        .request_node_shutdown(&node_name)
        .await
        .expect("request node shutdown");

    let row: (
        Option<i32>,
        Option<String>,
        Option<i32>,
        Option<String>,
        bool,
    ) = sqlx::query_as(
        r#"
        SELECT
            desired_run_id,
            desired_role,
            active_run_id,
            active_role,
            shutdown_requested_at IS NOT NULL
        FROM nodes
        WHERE name = $1
        "#,
    )
    .bind(&node_name)
    .fetch_one(store.pool())
    .await
    .expect("load node row");

    assert_eq!(row.0, None);
    assert_eq!(row.1, None);
    assert_eq!(row.2, Some(run_id));
    assert_eq!(row.3.as_deref(), Some("sampler_aggregator"));
    assert!(row.4);

    store.remove_run(run_id).await.expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn expired_shutdown_request_does_not_affect_replacement_node() {
    let (_test_guard, store) = locked_test_store().await;
    let node_name = unique_id("shutdown-replacement-node");
    let old_uuid = unique_id("shutdown-replacement-old");
    let new_uuid = unique_id("shutdown-replacement-new");

    store
        .announce_node(&node_name, &old_uuid, &Default::default())
        .await
        .expect("announce old node");
    store
        .request_node_shutdown(&node_name)
        .await
        .expect("request old node shutdown");
    store
        .expire_node_lease(&old_uuid)
        .await
        .expect("expire old node");
    store
        .announce_node(&node_name, &new_uuid, &Default::default())
        .await
        .expect("announce replacement node");

    let shutdown_requested = store
        .consume_node_shutdown_request(&new_uuid)
        .await
        .expect("consume replacement shutdown request");
    assert!(!shutdown_requested);
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn shutdown_all_nodes_clears_desired_assignments() {
    let (_test_guard, store) = locked_test_store().await;
    let node_a = unique_id("shutdown-all-a");
    let node_b = unique_id("shutdown-all-b");
    let uuid_a = unique_id("shutdown-all-uuid-a");
    let uuid_b = unique_id("shutdown-all-uuid-b");

    let run_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (
            name,
            integration_params,
            point_spec
        ) VALUES (
            'shutdown-all-clears-desired-run',
            '{}'::jsonb,
            '{"continuous_dims":0,"discrete_dims":0}'::jsonb
        )
        RETURNING id
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("insert run");

    store
        .announce_node(&node_a, &uuid_a, &Default::default())
        .await
        .expect("announce node a");
    store
        .announce_node(&node_b, &uuid_b, &Default::default())
        .await
        .expect("announce node b");
    store
        .assign_worker_pool(&node_a, WorkerRole::SamplerAggregator, run_id)
        .await
        .expect("assign desired sampler");
    store
        .assign_worker_pool(&node_b, WorkerRole::Evaluator, run_id)
        .await
        .expect("assign desired evaluator");

    let shutdown_targets: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM nodes
        WHERE lease_expires_at > now() OR resume_requested
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("count live and retained shutdown targets");
    let rows_updated = store
        .request_all_nodes_shutdown()
        .await
        .expect("request all node shutdown");
    assert_eq!(rows_updated, shutdown_targets as u64);

    let desired_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM nodes
        WHERE name = ANY($1)
          AND (desired_run_id IS NOT NULL OR pool_run_id IS NOT NULL)
        "#,
    )
    .bind(&[node_a, node_b])
    .fetch_one(store.pool())
    .await
    .expect("count desired assignments");
    assert_eq!(desired_count, 0);

    store.remove_run(run_id).await.expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn sampler_aggregator_current_assignment_is_unique_per_run() {
    let (_test_guard, store) = locked_test_store().await;
    let node_a = unique_id("node-a");
    let node_b = unique_id("node-b");
    let uuid_a = unique_id("uuid-a");
    let uuid_b = unique_id("uuid-b");

    let run_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (
            name,
            integration_params,
            point_spec
        ) VALUES (
            'test-run-current-sampler',
            '{}'::jsonb,
            '{"continuous_dims":0,"discrete_dims":0}'::jsonb
        )
        RETURNING id
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("insert run");

    store
        .announce_node(&node_a, &uuid_a, &Default::default())
        .await
        .expect("announce node a");
    store
        .announce_node(&node_b, &uuid_b, &Default::default())
        .await
        .expect("announce node b");

    store
        .set_current_assignment(&uuid_a, WorkerRole::SamplerAggregator, run_id)
        .await
        .expect("set current sampler on node a");

    let err = store
        .set_current_assignment(&uuid_b, WorkerRole::SamplerAggregator, run_id)
        .await
        .expect_err("second current sampler should fail");

    match err {
        StoreError::InvalidInput(message) => {
            assert!(
                message.contains("current sampler_aggregator"),
                "unexpected error message: {message}"
            );
        }
        other => panic!("expected invalid input, got {other}"),
    }

    store.remove_run(run_id).await.expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn prefetch_can_claim_fresh_work_while_a_peer_is_idle() {
    let (_guard, store) = locked_test_store().await;
    let (run, _, active, ids) = reliability_fixture(&store, 3).await;
    let idle = unique_id("idle-peer");
    store
        .announce_node(&idle, &idle, &Default::default())
        .await
        .unwrap();
    store
        .set_current_assignment(&idle, WorkerRole::Evaluator, run)
        .await
        .unwrap();
    // Keep rows fresh independently of test execution time.
    sqlx::query("UPDATE batches SET created_at=now()+interval '1 hour' WHERE run_id=$1")
        .bind(run)
        .execute(store.pool())
        .await
        .unwrap();
    let mut claimed = Vec::new();
    for node in [&active, &active, &idle] {
        let claim = store
            .claim_batch(run, node, &unique_id("claim"))
            .await
            .unwrap()
            .expect("available work must not wait for a peer to claim its first batch");
        claimed.push(claim.batch_id);
    }
    assert_eq!(claimed, ids);
    assert!(
        store
            .claim_batch(run, &active, &unique_id("claim"))
            .await
            .unwrap()
            .is_none()
    );
    store.remove_run(run).await.unwrap();
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn sampler_assignment_reserves_extra_connections_until_the_old_role_stops() {
    use gammaboard::core::{DesiredAssignment, NodeAssignmentUpdate};
    let (_guard, store) = locked_test_store().await;
    let capacity: i64 = sqlx::query_scalar(
        "SELECT (current_setting('max_connections')::bigint -
            current_setting('superuser_reserved_connections')::bigint -
            COALESCE(current_setting('reserved_connections',true)::bigint,0) - 16) / 4",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    let launch = store
        .reserve_worker_launch(
            "external",
            vec![serde_json::json!({
                "count": capacity, "name_prefix": unique_id("pool-budget")
            })],
        )
        .await
        .unwrap();
    let name: String = sqlx::query_scalar(
        "SELECT name FROM nodes WHERE launch_request_id=$1 ORDER BY name LIMIT 1",
    )
    .bind(launch)
    .fetch_one(store.pool())
    .await
    .unwrap();
    store
        .announce_node(&name, &name, &Default::default())
        .await
        .unwrap();
    let run: i32 = sqlx::query_scalar("INSERT INTO runs (name,integration_params,point_spec) VALUES ('pool-budget','{}','{\"continuous\":{\"dims\":0}}') RETURNING id")
        .fetch_one(store.pool()).await.unwrap();
    let sampler = DesiredAssignment {
        node_name: name.clone(),
        role: WorkerRole::SamplerAggregator,
        run_id: run,
        run_name: None,
    };
    // Both operator assignment and controller placement must roll back when
    // the additional sampler connections would spend the launch reservations.
    assert!(
        store
            .assign_worker_pool(&name, WorkerRole::SamplerAggregator, run)
            .await
            .unwrap_err()
            .to_string()
            .contains("connection budget exceeded")
    );
    assert!(
        store
            .update_desired_assignments(&[NodeAssignmentUpdate {
                node_uuid: name.clone(),
                expected: None,
                desired: Some(sampler),
            }])
            .await
            .unwrap_err()
            .to_string()
            .contains("connection budget exceeded")
    );
    assert!(store.get_desired_assignment(&name).await.unwrap().is_none());
    store
        .assign_worker_pool(&name, WorkerRole::Evaluator, run)
        .await
        .unwrap();

    // Releasing one four-connection reservation makes room for the sampler's
    // four extra connections without reducing the evaluator fleet allowance.
    sqlx::query("DELETE FROM nodes WHERE name=(SELECT name FROM nodes WHERE launch_request_id=$1 AND name<>$2 LIMIT 1)")
        .bind(launch).bind(&name).execute(store.pool()).await.unwrap();
    store
        .assign_worker_pool(&name, WorkerRole::SamplerAggregator, run)
        .await
        .unwrap();
    sqlx::query("UPDATE nodes SET active_run_id=$2,active_role='sampler_aggregator' WHERE name=$1")
        .bind(&name)
        .bind(run)
        .execute(store.pool())
        .await
        .unwrap();
    store
        .assign_worker_pool(&name, WorkerRole::Evaluator, run)
        .await
        .unwrap();
    assert!(
        store
            .reserve_worker_launch("external", vec![serde_json::json!({"count":1})])
            .await
            .unwrap_err()
            .to_string()
            .contains("connection budget exceeded"),
        "the old sampler still owns its role pool during reassignment"
    );
    sqlx::query("UPDATE nodes SET active_run_id=NULL,active_role=NULL WHERE name=$1")
        .bind(&name)
        .execute(store.pool())
        .await
        .unwrap();
    store
        .reserve_worker_launch("external", vec![serde_json::json!({"count":1})])
        .await
        .expect("sampler shutdown releases the extra reservation");
    store.remove_run(run).await.unwrap();
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn expired_launch_history_does_not_reserve_connections_or_require_simultaneous_leases() {
    let (_guard, store) = locked_test_store().await;
    let capacity: i64 = sqlx::query_scalar(
        "SELECT (current_setting('max_connections')::bigint -
            current_setting('superuser_reserved_connections')::bigint -
            COALESCE(current_setting('reserved_connections',true)::bigint,0) - 16) / 4",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    let prefix = unique_id("expired-launch");
    let old = store
        .reserve_worker_launch(
            "local",
            vec![serde_json::json!({
                "count":capacity,"name_prefix":prefix,
            })],
        )
        .await
        .unwrap();
    // Every worker connected and exited before the launch's final status was
    // recorded. Together these used to consume the entire admission budget.
    sqlx::query("UPDATE node_launch_requests SET state='starting',started_count=requested_count WHERE id=$1")
        .bind(old).execute(store.pool()).await.unwrap();
    sqlx::query("UPDATE nodes SET uuid=name,last_seen=now(),lease_expires_at=now()-interval '1 second' WHERE launch_request_id=$1")
        .bind(old).execute(store.pool()).await.unwrap();
    let new = store
        .reserve_worker_launch(
            "external",
            vec![serde_json::json!({
                "count":capacity,"name_prefix":prefix,
            })],
        )
        .await
        .expect(
            "expired workers must not consume admission capacity before request reconciliation",
        );
    let next_name: String = sqlx::query_scalar(
        "SELECT name FROM nodes WHERE launch_request_id=$1 ORDER BY name LIMIT 1",
    )
    .bind(new)
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert!(
        next_name
            .strip_prefix(&format!("{prefix}-"))
            .unwrap()
            .parse::<i64>()
            .unwrap()
            > capacity,
        "new launches preserve historical names rather than reusing them"
    );
    assert!(
        store
            .reserve_worker_launch(
                "local",
                vec![serde_json::json!({
                    "count":1,"name_prefix":prefix,
                })]
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("awaiting registration"),
        "unannounced external workers must still reserve capacity"
    );
    let requests = store.list_node_launch_requests().await.unwrap();
    assert_eq!(
        requests.iter().find(|r| r.id == old).unwrap().state,
        "fulfilled"
    );
    assert_eq!(
        requests.iter().find(|r| r.id == new).unwrap().state,
        "pending"
    );
    // One worker in a partially started group connected and then stopped;
    // release just its reservation while keeping the other workers reserved.
    sqlx::query("UPDATE node_launch_requests SET state='starting',started_count=requested_count WHERE id=$1")
        .bind(new).execute(store.pool()).await.unwrap();
    sqlx::query("UPDATE nodes SET uuid=name,last_seen=now() WHERE name=$1")
        .bind(&next_name)
        .execute(store.pool())
        .await
        .unwrap();
    let partial = store
        .reserve_worker_launch(
            "local",
            vec![serde_json::json!({
                "count":1,"name_prefix":prefix,
            })],
        )
        .await
        .expect("an expired, already connected worker is no longer a reservation");
    let ids = [old, new, partial];
    sqlx::query("DELETE FROM nodes WHERE launch_request_id=ANY($1)")
        .bind(ids.as_slice())
        .execute(store.pool())
        .await
        .unwrap();
    sqlx::query("DELETE FROM node_launch_requests WHERE id=ANY($1)")
        .bind(ids.as_slice())
        .execute(store.pool())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn resumed_worker_requires_a_new_registration_before_its_launch_is_fulfilled() {
    let (_guard, store) = locked_test_store().await;
    let prefix = unique_id("resume-registration");
    let old = store
        .reserve_worker_launch(
            "local",
            vec![serde_json::json!({"count":1,"name_prefix":prefix})],
        )
        .await
        .unwrap();
    let name = format!("{prefix}-1");
    store
        .announce_node(&name, &unique_id("old-process"), &Default::default())
        .await
        .unwrap();
    sqlx::query("UPDATE node_launch_requests SET state='fulfilled',started_count=1 WHERE id=$1")
        .bind(old)
        .execute(store.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE nodes SET resume_requested=true,lease_expires_at=now()-interval '1 second' WHERE name=$1")
        .bind(&name).execute(store.pool()).await.unwrap();
    assert_eq!(store.enqueue_resumed_workers().await.unwrap(), 1);
    let new: i64 = sqlx::query_scalar("SELECT launch_request_id FROM nodes WHERE name=$1")
        .bind(&name)
        .fetch_one(store.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE node_launch_requests SET state='starting',started_count=1 WHERE id=$1")
        .bind(new)
        .execute(store.pool())
        .await
        .unwrap();
    let requests = store.list_node_launch_requests().await.unwrap();
    assert_eq!(
        requests.iter().find(|r| r.id == new).unwrap().state,
        "starting",
        "the old process's heartbeat must not fulfill the replacement launch"
    );
    store
        .announce_node(
            &name,
            &unique_id("replacement-process"),
            &Default::default(),
        )
        .await
        .unwrap();
    let requests = store.list_node_launch_requests().await.unwrap();
    assert_eq!(
        requests.iter().find(|r| r.id == new).unwrap().state,
        "fulfilled"
    );
    sqlx::query("DELETE FROM nodes WHERE name=$1")
        .bind(&name)
        .execute(store.pool())
        .await
        .unwrap();
    sqlx::query("DELETE FROM node_launch_requests WHERE id=ANY($1)")
        .bind([old, new].as_slice())
        .execute(store.pool())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn launch_history_preserves_outstanding_requests_and_worker_identity() {
    let (_test_guard, store) = locked_test_store().await;
    let prefix = unique_id("launch-history");
    let request_id = store
        .reserve_worker_launch(
            "external",
            vec![serde_json::json!({
                "count": 1, "name_prefix": prefix,
            })],
        )
        .await
        .unwrap();
    let name = format!("{prefix}-1");
    let result = serde_json::json!({"workers": [{"node_name": name}]});
    sqlx::query(
        "UPDATE node_launch_requests SET state='starting',started_count=1,result=$2 WHERE id=$1",
    )
    .bind(request_id)
    .bind(&result)
    .execute(store.pool())
    .await
    .unwrap();
    // Submission alone does not fulfill a request.
    let requests = store.list_node_launch_requests().await.unwrap();
    assert_eq!(
        requests.iter().find(|r| r.id == request_id).unwrap().state,
        "starting"
    );
    store
        .announce_node(&name, &unique_id("launch-worker"), &Default::default())
        .await
        .unwrap();
    let state: String = sqlx::query_scalar("SELECT state FROM node_launch_requests WHERE id=$1")
        .bind(request_id)
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(
        state, "fulfilled",
        "success is recorded without polling the request list"
    );
    let requests = store.list_node_launch_requests().await.unwrap();
    assert_eq!(
        requests.iter().find(|r| r.id == request_id).unwrap().state,
        "fulfilled"
    );
    store.suspend_workers().await.unwrap();
    sqlx::query("UPDATE nodes SET lease_expires_at=now()-interval '1 minute' WHERE name=$1")
        .bind(&name)
        .execute(store.pool())
        .await
        .unwrap();
    assert_eq!(store.enqueue_resumed_workers().await.unwrap(), 1);
    let replacement: i64 = sqlx::query_scalar("SELECT launch_request_id FROM nodes WHERE name=$1")
        .bind(&name)
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_ne!(replacement, request_id);
    let requests = store.list_node_launch_requests().await.unwrap();
    assert_eq!(
        requests.iter().find(|r| r.id == request_id).unwrap().state,
        "fulfilled",
        "worker shutdown does not change the historical launch outcome"
    );
    let node_count: i64 = sqlx::query_scalar("SELECT count(*) FROM nodes WHERE name=$1")
        .bind(&name)
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(node_count, 1);
    // A replacement's live lease cannot fulfill an earlier, incomplete attempt.
    sqlx::query("UPDATE node_launch_requests SET state='starting' WHERE id=$1")
        .bind(request_id)
        .execute(store.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE nodes SET lease_expires_at=now()+interval '1 minute' WHERE name=$1")
        .bind(&name)
        .execute(store.pool())
        .await
        .unwrap();
    let history_ids: Vec<i64> = sqlx::query_scalar("INSERT INTO node_launch_requests (state,backend,requested_count) SELECT 'fulfilled','external',1 FROM generate_series(1,101) RETURNING id")
        .fetch_all(store.pool()).await.unwrap();
    let requests = store.list_node_launch_requests().await.unwrap();
    assert_eq!(
        requests.iter().find(|r| r.id == request_id).unwrap().state,
        "starting"
    );
    assert_eq!(
        requests.iter().find(|r| r.id == replacement).unwrap().state,
        "pending"
    );
    assert_eq!(
        requests.iter().filter(|r| r.state == "fulfilled").count(),
        100
    );
    sqlx::query("DELETE FROM nodes WHERE name=$1")
        .bind(name)
        .execute(store.pool())
        .await
        .unwrap();
    let mut ids = history_ids;
    ids.extend([request_id, replacement]);
    sqlx::query("DELETE FROM node_launch_requests WHERE id=ANY($1)")
        .bind(ids)
        .execute(store.pool())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn completed_cleanup_preserves_committed_checkpoint_work() {
    let (_guard, store) = locked_test_store().await;
    let run: i32 = sqlx::query_scalar("INSERT INTO runs (name,integration_params,point_spec) VALUES ('checkpoint-cleanup','{}','{\"continuous\":{\"dims\":1}}') RETURNING id")
        .fetch_one(store.pool()).await.unwrap();
    let task = insert_completed_pause_task(&store, run).await;
    let batch = Batch::from_points([Point::new(vec![1.0], vec![], 1.0)]).unwrap();
    let ids = next_batch_ids(5);
    store
        .insert_batches(
            run,
            task,
            false,
            &ids,
            &vec![LatentBatchSpec::from_batch(&batch).build(); 5],
        )
        .await
        .unwrap();
    sqlx::query("UPDATE batches SET status='completed' WHERE run_id=$1")
        .bind(run)
        .execute(store.pool())
        .await
        .unwrap();
    assert_eq!(
        store
            .cleanup_consumed_completed_batches(run, ids[4], 100)
            .await
            .unwrap(),
        0,
        "without a checkpoint every result must remain replayable"
    );
    sqlx::query(
        "INSERT INTO run_sampler_checkpoints (run_id,task_id,sampler_checkpoint) VALUES ($1,$2,$3)",
    )
    .bind(run)
    .bind(task)
    .bind(serde_json::json!({"queue":{"last_completed_batch_id":ids[0],"last_produced_batch_id":ids[2]}}))
    .execute(store.pool())
    .await
    .unwrap();
    assert_eq!(
        store
            .cleanup_consumed_completed_batches(run, ids[4], 100)
            .await
            .unwrap(),
        3
    );
    let mut tx = store.pool().begin().await.unwrap();
    sqlx::query("UPDATE run_sampler_checkpoints SET sampler_checkpoint=$2 WHERE run_id=$1")
        .bind(run)
        .bind(serde_json::json!({"queue":{"last_completed_batch_id":ids[1],"last_produced_batch_id":ids[2]}}))
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(
        store
            .cleanup_consumed_completed_batches(run, ids[4], 100)
            .await
            .unwrap(),
        0,
        "an in-flight checkpoint must not authorize cleanup"
    );
    tx.rollback().await.unwrap();
    assert_eq!(
        store
            .cleanup_consumed_completed_batches(run, ids[4], 100)
            .await
            .unwrap(),
        0,
        "a failed checkpoint must not authorize cleanup"
    );
    sqlx::query("UPDATE run_sampler_checkpoints SET sampler_checkpoint=$2 WHERE run_id=$1")
        .bind(run)
        .bind(serde_json::json!({"queue":{"last_completed_batch_id":ids[1],"last_produced_batch_id":ids[2]}}))
        .execute(store.pool())
        .await
        .unwrap();
    assert_eq!(
        store
            .cleanup_consumed_completed_batches(run, ids[4], 100)
            .await
            .unwrap(),
        1
    );
    let remaining: Vec<i64> = sqlx::query_scalar("SELECT id FROM batches WHERE run_id=$1")
        .bind(run)
        .fetch_all(store.pool())
        .await
        .unwrap();
    assert_eq!(remaining, vec![ids[2]]);
    store.remove_run(run).await.unwrap();
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn checkpoint_and_stage_publish_atomically_after_schema_compaction() {
    use gammaboard::core::{AggregationStore, RunStageSnapshot, SamplerAggregatorCheckpoint};
    use gammaboard::evaluation::AccumulatorState;
    use gammaboard::sampling::SamplerAggregatorSnapshot;
    use serde_json::json;

    let (_guard, store) = locked_test_store().await;
    let run: i32 = sqlx::query_scalar("INSERT INTO runs (name,integration_params,point_spec) VALUES ('checkpoint-atomic','{}','{\"continuous\":{\"dims\":1}}') RETURNING id")
        .fetch_one(store.pool()).await.unwrap();
    let task = insert_completed_pause_task(&store, run).await;
    sqlx::query("UPDATE run_tasks SET state='active' WHERE id=$1")
        .bind(task)
        .execute(store.pool())
        .await
        .unwrap();
    let sampler = SamplerAggregatorSnapshot::NaiveMonteCarlo { raw: json!({}) };
    let observable = AccumulatorState::empty_scalar();
    // A checkpoint written by the preceding version, including discarded live metrics.
    let old = json!({
        "completed_samples":0, "task_id":task, "output_snapshot_id":null, "batches_completed":null,
        "sampler_snapshot":sampler, "observable_state":observable,
        "runtime_state":{
            "produced_batches_total":0, "produced_samples_total":0,
            "ingested_batches_total":0, "ingested_samples_total":0,
            "sampler_uptime_ms_accumulated":10.0, "accumulator_checkpoint_state":"NeedsInitialRoundTrip",
            "completed_samples_per_second":99.0, "eta_seconds":1.0, "sampler_tick_busy_ratio":0.8,
            "initial_round_trip_snapshot_pending":false, "pending_persisted_completed_batches":0,
            "batch_size_current":128
        },
        "queue":{"last_completed_batch_id":null,"last_produced_batch_id":null,"batch_size_current":128}
    });
    sqlx::query(
        "INSERT INTO run_sampler_checkpoints (run_id,task_id,sampler_checkpoint) VALUES ($1,$2,$3)",
    )
    .bind(run)
    .bind(task)
    .bind(old)
    .execute(store.pool())
    .await
    .unwrap();
    sqlx::raw_sql(include_str!(
        "../migrations/202609110004_compact_checkpoints.sql"
    ))
    .execute(store.pool())
    .await
    .unwrap();
    let mut checkpoint: SamplerAggregatorCheckpoint =
        store.load_sampler_checkpoint(run).await.unwrap().unwrap();
    let mut stale_checkpoint = checkpoint.clone();
    stale_checkpoint.completed_samples += 1;
    let conflict = store
        .restore_sampler_checkpoint(run, &stale_checkpoint)
        .await
        .expect_err("stale checkpoint must not restore");
    assert!(
        conflict.is_retry_activation(),
        "stale checkpoint should request fresh runtime activation: {conflict}"
    );
    store
        .restore_sampler_checkpoint(run, &checkpoint)
        .await
        .expect("migrated checkpoint remains restorable");
    let compact = serde_json::to_value(&checkpoint).unwrap();
    assert_eq!(compact["runtime_state"].as_object().unwrap().len(), 6);
    assert_eq!(compact["batches_completed"], 0);
    let stage = RunStageSnapshot {
        id: None,
        run_id: run,
        task_id: Some(task),
        name: "saved".into(),
        sequence_nr: Some(0),
        queue_empty: true,
        sampler_snapshot: Some(sampler),
        observable_state: Some(observable),
        evaluator: None,
        sampler_aggregator: None,
        batch_transforms: vec![],
    };
    store
        .save_sampler_checkpoint(run, &checkpoint, Some(&stage))
        .await
        .unwrap();
    let before = store.load_sampler_checkpoint(run).await.unwrap().unwrap();
    // Fail after the stage INSERT, inside the checkpoint transaction.
    let constraint = format!("reject_test_checkpoint_{run}");
    sqlx::query(&format!("ALTER TABLE run_sampler_checkpoints ADD CONSTRAINT {constraint} CHECK (run_id <> {run} OR (sampler_checkpoint->>'completed_samples')::bigint < 1000)"))
        .execute(store.pool()).await.unwrap();
    checkpoint.completed_samples = 1000;
    assert!(
        store
            .save_sampler_checkpoint(run, &checkpoint, Some(&stage))
            .await
            .is_err()
    );
    sqlx::query(&format!(
        "ALTER TABLE run_sampler_checkpoints DROP CONSTRAINT {constraint}"
    ))
    .execute(store.pool())
    .await
    .unwrap();
    let stages: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_stage_snapshots WHERE run_id=$1")
            .bind(run)
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(
        stages, 1,
        "failed save cannot leave an orphan stage snapshot"
    );
    let after = store.load_sampler_checkpoint(run).await.unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(after).unwrap(),
        serde_json::to_value(before).unwrap()
    );
    let status = store.checkpoint_status(run).await.unwrap();
    assert_eq!(status["saved_samples"], 0);
    // A mismatched embedded task must fail instead of silently changing recovery identity.
    sqlx::query("UPDATE run_sampler_checkpoints SET sampler_checkpoint=jsonb_set(sampler_checkpoint,'{task_id}','-1') WHERE run_id=$1")
        .bind(run).execute(store.pool()).await.unwrap();
    assert!(store.load_sampler_checkpoint(run).await.is_err());
    store.remove_run(run).await.unwrap();
}

// Regression fixtures for task isolation and evaluator persistence failures.
async fn reliability_fixture(store: &PgStore, count: usize) -> (i32, i64, String, Vec<i64>) {
    let run: i32 = sqlx::query_scalar("INSERT INTO runs (name,integration_params,point_spec) VALUES ('queue-reliability','{}','{\"rectangular\":{\"continuous_dims\":1,\"discrete_cardinalities\":[]}}') RETURNING id")
        .fetch_one(store.pool()).await.unwrap();
    let task: RunTaskSpec = serde_json::from_value(serde_json::json!({
        "kind":"sample", "stop_condition":{"max_samples":100},
        "evaluator":{"config":{"kind":"unit"}},
        "sampler_aggregator":{"config":{"kind":"naive_monte_carlo"}}
    }))
    .unwrap();
    let task_id = store
        .append_run_tasks(
            run,
            &[RunTaskInput {
                name: Some("sample".into()),
                task,
            }],
        )
        .await
        .unwrap()[0]
        .id;
    store.activate_next_run_task(run).await.unwrap();
    let node = unique_id("reliability-node");
    store
        .announce_node(&node, &node, &Default::default())
        .await
        .unwrap();
    store
        .set_current_assignment(&node, WorkerRole::Evaluator, run)
        .await
        .unwrap();
    let batch = Batch::from_points([Point::new(vec![0.5], Vec::new(), 1.0)]).unwrap();
    let ids = next_batch_ids(count);
    store
        .insert_batches(
            run,
            task_id,
            false,
            &ids,
            &vec![LatentBatchSpec::from_batch(&batch).build(); count],
        )
        .await
        .unwrap();
    (run, task_id, node, ids)
}

fn empty_batch_result() -> gammaboard::evaluation::BatchResult {
    gammaboard::evaluation::BatchResult::new(
        None,
        gammaboard::evaluation::AccumulatorState::Empty(Default::default()),
    )
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn telemetry_does_not_lock_progress_and_still_cascades() {
    let (_guard, store) = locked_test_store().await;
    let (run, task, node, _) = reliability_fixture(&store, 0).await;
    sqlx::query("INSERT INTO run_telemetry_workers (run_id,worker_id) VALUES ($1,$2)")
        .bind(run)
        .bind(&node)
        .execute(store.pool())
        .await
        .unwrap();
    let mut writer = store.pool().begin().await.unwrap();
    sqlx::query("SELECT id FROM runs WHERE id=$1 FOR UPDATE")
        .bind(run)
        .fetch_one(&mut *writer)
        .await
        .unwrap();
    let tables = [
        "evaluator_performance_history",
        "evaluator_performance_latest",
        "sampler_aggregator_performance_history",
        "sampler_aggregator_performance_latest",
    ];
    tokio::time::timeout(Duration::from_secs(2), async {
        for table in tables {
            sqlx::query(&format!(
                "WITH owner AS (INSERT INTO run_telemetry_workers VALUES ($1,$2) ON CONFLICT DO NOTHING)
                 INSERT INTO {table} (id,run_id,worker_id) VALUES (0,$1,$2)"
            ))
            .bind(run)
            .bind(&node)
            .execute(store.pool())
            .await
            .unwrap();
        }
    })
    .await
    .expect("telemetry must not lock the mutable run row");
    writer.rollback().await.unwrap();
    store.expire_node_lease(&node).await.unwrap();
    store.remove_run(run).await.unwrap();
    for table in tables
        .into_iter()
        .chain(["run_telemetry_workers", "task_worker_cpu_time"])
    {
        let (column, id) = if table == "task_worker_cpu_time" {
            ("task_id", task)
        } else {
            ("run_id", i64::from(run))
        };
        let remaining: i64 =
            sqlx::query_scalar(&format!("SELECT count(*) FROM {table} WHERE {column}=$1"))
                .bind(id)
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(remaining, 0, "run deletion must cascade through {table}");
    }
    assert!(
        sqlx::query("INSERT INTO evaluator_performance_history (run_id,worker_id) VALUES ($1,$2)")
            .bind(run)
            .bind(&node)
            .execute(store.pool())
            .await
            .is_err(),
        "missing runs must still be rejected"
    );
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn idle_evaluator_publishes_current_task_without_receiving_work() {
    use gammaboard::core::EvaluatorConfig;
    use gammaboard::runners::{EvaluatorRunner, EvaluatorRunnerParams};
    let (_guard, store) = locked_test_store().await;
    let (run, task, node, _) = reliability_fixture(&store, 0).await;
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut runner = EvaluatorRunner::new(
        store.clone(),
        run,
        &node,
        &node,
        EvaluatorConfig::Unit {
            params: Default::default(),
        },
        Box::new(CountingEvaluator(calls.clone())),
        gammaboard::Domain::rectangular(1, 0),
        EvaluatorRunnerParams {
            performance_snapshot_interval_ms: 0,
        },
        3,
    );
    runner.tick().await.unwrap();
    let first = gammaboard::api::performance::snapshot(&store, run)
        .await
        .unwrap();
    let metrics = &first.evaluators[0]["metrics"];
    assert_eq!(metrics["task_id"], task.to_string());
    assert_eq!(metrics["samples_evaluated"], 0);
    assert_eq!(metrics["busy"]["compute_seconds"], 0.0);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    store.complete_run_task(task).await.unwrap();
    let config = store.load_run_task(task).await.unwrap().unwrap().task;
    let next = store
        .append_run_tasks(
            run,
            &[RunTaskInput {
                name: Some("next".into()),
                task: config,
            }],
        )
        .await
        .unwrap()[0]
        .id;
    store.activate_next_run_task(run).await.unwrap();
    runner.tick().await.unwrap();
    let second = gammaboard::api::performance::snapshot(&store, run)
        .await
        .unwrap();
    assert_eq!(second.evaluators[0]["metrics"]["task_id"], next.to_string());
    assert_eq!(second.evaluators[0]["metrics"]["epoch"], metrics["epoch"]);
    // Exercise the idle backoff, then ensure a newly populated queue is served.
    for _ in 0..20 {
        runner.tick().await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let batch = Batch::from_points([Point::new(vec![0.5], Vec::new(), 1.0)]).unwrap();
    let ids = next_batch_ids(8);
    store
        .insert_batches(
            run,
            next,
            false,
            &ids,
            &vec![LatentBatchSpec::from_batch(&batch).build(); 8],
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            runner.tick().await.unwrap();
            let completed: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM batches WHERE run_id=$1 AND status='completed'",
            )
            .bind(run)
            .fetch_one(store.pool())
            .await
            .unwrap();
            if completed == 8 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("idle backoff must not strand newly queued batches");
    runner.stop().await.unwrap();
    store.expire_node_lease(&node).await.unwrap();
    store.remove_run(run).await.unwrap();
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn shutdown_deadline_includes_the_initial_database_update() {
    let (_guard, store) = locked_test_store().await;
    let node = unique_id("shutdown-lock");
    store
        .announce_node(&node, &node, &Default::default())
        .await
        .unwrap();
    let mut writer = store.pool().begin().await.unwrap();
    sqlx::query("SELECT name FROM nodes WHERE name=$1 FOR UPDATE")
        .bind(&node)
        .fetch_one(&mut *writer)
        .await
        .unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        gammaboard::api::nodes::suspend_nodes_gracefully(
            &store,
            gammaboard::api::nodes::GracefulNodeShutdownParams {
                sampler_drain_timeout_seconds: 1,
                node_stop_timeout_seconds: 0,
                poll_interval_ms: 10,
            },
        ),
    )
    .await
    .expect("outer shutdown deadline must bound blocked SQL");
    assert!(result.unwrap_err().to_string().contains("exceeded 1s"));
    writer.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn leader_lookup_tracks_lease_expiry_and_worker_replacement() {
    let (_guard, store) = locked_test_store().await;
    store
        .announce_node("a", "a1", &Default::default())
        .await
        .unwrap();
    store
        .announce_node("b", "b1", &Default::default())
        .await
        .unwrap();
    assert!(store.is_task_control_leader("a", "a1").await.unwrap());
    assert!(!store.is_task_control_leader("b", "b1").await.unwrap());
    store.expire_node_lease("a1").await.unwrap();
    assert!(store.is_task_control_leader("b", "b1").await.unwrap());
    store
        .announce_node("a", "a2", &Default::default())
        .await
        .unwrap();
    assert!(!store.is_task_control_leader("a", "a1").await.unwrap());
    assert!(store.is_task_control_leader("a", "a2").await.unwrap());
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn campaign_stall_checkpoint_jsonb_number_roundtrip_preserves_recovery() {
    use gammaboard::core::{AggregationStore, SamplerAggregatorCheckpoint};
    use gammaboard::evaluation::AccumulatorState;
    use gammaboard::sampling::SamplerAggregatorSnapshot;
    use serde_json::{Value, json};

    let (_guard, store) = locked_test_store().await;
    let (run, task, node, ids) = reliability_fixture(&store, 2).await;
    for _ in 0..2 {
        let token = unique_id("checkpoint-claim");
        let batch = store
            .claim_batch(run, &node, &token)
            .await
            .unwrap()
            .unwrap();
        store
            .submit_batch_results(batch.batch_id, &node, &token, &empty_batch_result(), 1.0)
            .await
            .unwrap();
    }
    // The actual Jacobian that prevented campaign 55's GL00 sampler from restarting.
    let mut point = Point::new(vec![0.5], vec![], 1.0);
    point.parameterization_jacobian = Some(7.553_746_958_286_027e18);
    point.add_weight_factor(
        "gammaloop_parameterization_jacobian",
        point.parameterization_jacobian.unwrap(),
    );
    let AccumulatorState::Vector(mut observable) = AccumulatorState::empty_scalar() else {
        unreachable!()
    };
    observable.ingest_vector(&[1.0], &point).unwrap();
    let checkpoint: SamplerAggregatorCheckpoint = serde_json::from_value(json!({
        "completed_samples": 1, "task_id": task, "output_snapshot_id": null, "batches_completed": 1,
        "sampler_snapshot": SamplerAggregatorSnapshot::NaiveMonteCarlo { raw: json!({"seed": 1}) },
        "observable_state": AccumulatorState::Vector(observable),
        "runtime_state": {
            "produced_batches_total": 3, "produced_samples_total": 3,
            "ingested_batches_total": 1, "ingested_samples_total": 1,
            "sampler_uptime_ms_accumulated": 10.0, "accumulator_checkpoint_state": "Ready"
        },
        "queue": {"last_completed_batch_id": ids[0] - 1, "last_produced_batch_id": ids[1], "batch_size_current": 1}
    })).unwrap();
    store
        .save_sampler_checkpoint(run, &checkpoint, None)
        .await
        .unwrap();
    let loaded = store.load_sampler_checkpoint(run).await.unwrap().unwrap();
    let raw: Value = sqlx::query_scalar(
        "SELECT sampler_checkpoint FROM run_sampler_checkpoints WHERE run_id=$1",
    )
    .bind(run)
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_ne!(
        raw,
        serde_json::to_value(&loaded).unwrap(),
        "fixture must reproduce the JSONB integer/f64 mismatch"
    );
    store
        .restore_sampler_checkpoint(run, &loaded)
        .await
        .expect("unchanged checkpoint must restore despite JSON number representation");
    let restored = store.get_run_progress(run).await.unwrap().unwrap();
    assert_eq!(
        (restored.nr_produced_samples, restored.nr_completed_samples),
        (3, 1)
    );
    let retained = store
        .fetch_completed_batches(run, task, 10, true, None)
        .await
        .unwrap();
    assert_eq!(
        retained
            .iter()
            .map(|batch| batch.batch_id)
            .collect::<Vec<_>>(),
        ids
    );

    // The fix must retain protection against both progress and sampler-state changes.
    let mut changed = loaded.clone();
    changed.completed_samples += 1;
    assert!(
        store
            .restore_sampler_checkpoint(run, &changed)
            .await
            .unwrap_err()
            .is_retry_activation()
    );
    let mut changed = loaded.clone();
    changed.sampler_snapshot = SamplerAggregatorSnapshot::NaiveMonteCarlo {
        raw: json!({"seed": 2}),
    };
    store
        .save_sampler_checkpoint(run, &changed, None)
        .await
        .unwrap();
    assert!(
        store
            .restore_sampler_checkpoint(run, &loaded)
            .await
            .unwrap_err()
            .is_retry_activation()
    );
    store.expire_node_lease(&node).await.unwrap();
    store.remove_run(run).await.unwrap();
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn campaign_stall_lifecycle_counts_only_live_worker_assignments() {
    use gammaboard::stores::RunLifecycleState;

    let (_guard, store) = locked_test_store().await;
    let parent: i32 = sqlx::query_scalar("INSERT INTO runs (name,integration_params,point_spec) VALUES ('lease-parent','{\"run_kind\":\"integration_campaign\"}','{}') RETURNING id")
        .fetch_one(store.pool()).await.unwrap();
    let mut children = Vec::new();
    let mut nodes = Vec::new();
    for index in 0..2 {
        let child: i32 = sqlx::query_scalar("INSERT INTO runs (name,parent_run_id,integration_params,point_spec) VALUES ($1,$2,'{}','{}') RETURNING id")
            .bind(format!("lease-child-{index}")).bind(parent).fetch_one(store.pool()).await.unwrap();
        let node = unique_id("lease-node");
        store
            .announce_node(&node, &node, &Default::default())
            .await
            .unwrap();
        sqlx::query("UPDATE nodes SET desired_run_id=$2,desired_role='evaluator',active_run_id=$2,active_role='evaluator',lease_expires_at=now()+interval '1 hour' WHERE name=$1")
            .bind(&node).bind(child).execute(store.pool()).await.unwrap();
        children.push(child);
        nodes.push(node);
    }
    // An expired worker retains both desired and active columns after an unclean exit.
    sqlx::query("UPDATE nodes SET lease_expires_at=now()-interval '1 second' WHERE name=$1")
        .bind(&nodes[0])
        .execute(store.pool())
        .await
        .unwrap();
    let stale = store.get_run_progress(children[0]).await.unwrap().unwrap();
    assert_eq!(stale.desired_assignment_count, 0);
    assert_eq!(stale.active_worker_count, 0);
    assert_eq!(stale.lifecycle_state, RunLifecycleState::Paused);
    let live = store.get_run_progress(children[1]).await.unwrap().unwrap();
    assert_eq!(
        (live.desired_assignment_count, live.active_worker_count),
        (1, 1)
    );
    assert_eq!(live.lifecycle_state, RunLifecycleState::Running);
    assert_eq!(
        store
            .get_run_progress(parent)
            .await
            .unwrap()
            .unwrap()
            .lifecycle_state,
        RunLifecycleState::Running
    );

    // A live draining worker is still pausing; expiry removes it from that count too.
    sqlx::query("UPDATE nodes SET desired_run_id=NULL,desired_role=NULL WHERE name=$1")
        .bind(&nodes[1])
        .execute(store.pool())
        .await
        .unwrap();
    assert_eq!(
        store
            .get_run_progress(parent)
            .await
            .unwrap()
            .unwrap()
            .lifecycle_state,
        RunLifecycleState::Pausing
    );
    sqlx::query("UPDATE nodes SET lease_expires_at=now()-interval '1 second' WHERE name=$1")
        .bind(&nodes[1])
        .execute(store.pool())
        .await
        .unwrap();
    assert_eq!(
        store
            .get_run_progress(parent)
            .await
            .unwrap()
            .unwrap()
            .lifecycle_state,
        RunLifecycleState::Paused
    );
    let listed = store.get_runs_page(500, 0, true).await.unwrap();
    assert!(
        listed
            .iter()
            .filter(|run| run.run_id == parent || children.contains(&run.run_id))
            .all(|run| run.lifecycle_state == RunLifecycleState::Paused)
    );
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM nodes WHERE name=ANY($1)")
        .bind(&nodes)
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(stored, 2, "lifecycle reads must preserve worker history");
    store.remove_run(parent).await.unwrap();
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn completed_fetch_and_counts_isolate_tasks_before_ordering_and_limit() {
    let (_guard, store) = locked_test_store().await;
    let (run, current_task, node, ids) = reliability_fixture(&store, 10).await;
    let old_task = insert_completed_pause_task(&store, run).await;
    sqlx::query("UPDATE batches SET task_id=$2,retry_count=1 WHERE id=ANY($1)")
        .bind(&ids[..6])
        .bind(old_task)
        .execute(store.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE batches SET status='completed' WHERE run_id=$1")
        .bind(run)
        .execute(store.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO batch_results (batch_id,batch_observable,completed_at) SELECT id,$2,now() FROM batches WHERE run_id=$1")
        .bind(run).bind(empty_batch_result().accumulator.to_json().unwrap()).execute(store.pool()).await.unwrap();
    // An old task's unfinished row must not block the new task. Nor may its
    // retained completed results consume the new task's fetch limit.
    sqlx::query("UPDATE batches SET status='claimed' WHERE id=$1")
        .bind(ids[0])
        .execute(store.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE batches SET status='pending' WHERE id=$1")
        .bind(ids[8])
        .execute(store.pool())
        .await
        .unwrap();
    for strict in [true, false] {
        let first = store
            .fetch_completed_batches(run, current_task, 1, strict, None)
            .await
            .unwrap();
        assert_eq!(
            first.iter().map(|b| b.batch_id).collect::<Vec<_>>(),
            vec![ids[6]]
        );
        let batches = store
            .fetch_completed_batches(run, current_task, 100, strict, None)
            .await
            .unwrap();
        assert_eq!(batches.len(), if strict { 2 } else { 3 });
        assert!(batches.iter().all(|b| b.task_id == current_task));
    }
    let counts = store
        .get_batch_queue_counts(run, Some(current_task), None)
        .await
        .unwrap();
    assert_eq!(
        (
            counts.pending,
            counts.claimed,
            counts.completed,
            counts.failed
        ),
        (1, 0, 3, 0)
    );
    let counts = store
        .get_batch_queue_counts(run, Some(current_task), Some(ids[7]))
        .await
        .unwrap();
    assert_eq!(counts.completed, 1);
    let blocker = store
        .get_queue_blocker(run, current_task, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(blocker.batch_id, ids[8]);
    assert_eq!(blocker.status, "pending");
    sqlx::query("DELETE FROM nodes WHERE uuid=$1")
        .bind(node)
        .execute(store.pool())
        .await
        .unwrap();
    store.remove_run(run).await.unwrap();
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn claim_and_submission_retries_are_idempotent_and_fence_old_generations() {
    let (_guard, store) = locked_test_store().await;
    let (run, _, node, ids) = reliability_fixture(&store, 2).await;
    let token = unique_id("claim");
    let first = store
        .claim_batch(run, &node, &token)
        .await
        .unwrap()
        .unwrap();
    let repeated = store
        .claim_batch(run, &node, &token)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        first.batch_id, repeated.batch_id,
        "lost claim acknowledgement must not claim another batch"
    );
    assert_eq!(first.batch_id, ids[0]);
    store
        .release_claimed_batches_for_worker(run, &node)
        .await
        .unwrap();
    let token2 = unique_id("replacement");
    let replacement = store
        .claim_batch(run, &node, &token2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(replacement.batch_id, first.batch_id);
    let result = empty_batch_result();
    assert!(
        store
            .submit_batch_results(first.batch_id, &node, &token, &result, 1.0)
            .await
            .unwrap_err()
            .is_batch_ownership_lost()
    );
    assert!(
        store
            .fail_batch(first.batch_id, &node, &token, "stale failure", 1)
            .await
            .unwrap_err()
            .is_batch_ownership_lost()
    );
    store
        .submit_batch_results(first.batch_id, &node, &token2, &result, 2.0)
        .await
        .unwrap();
    store
        .submit_batch_results(first.batch_id, &node, &token2, &result, 99.0)
        .await
        .unwrap();
    let (count, duration): (i64, f64) = sqlx::query_as(
        "SELECT count(*),min(total_eval_time_ms) FROM batch_results WHERE batch_id=$1",
    )
    .bind(first.batch_id)
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(
        (count, duration),
        (1, 2.0),
        "retry must preserve the first accepted result"
    );
    let token3 = unique_id("failure");
    let failure = store
        .claim_batch(run, &node, &token3)
        .await
        .unwrap()
        .unwrap();
    for _ in 0..2 {
        assert!(matches!(
            store
                .fail_batch(failure.batch_id, &node, &token3, "failure", 3)
                .await
                .unwrap(),
            BatchFailOutcome::Requeued { retry_count: 1, .. }
        ));
    }
    sqlx::query("DELETE FROM nodes WHERE uuid=$1")
        .bind(node)
        .execute(store.pool())
        .await
        .unwrap();
    store.remove_run(run).await.unwrap();
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn claim_reconciliation_preserves_slow_work_and_releases_only_untracked_claims() {
    let (_guard, store) = locked_test_store().await;
    let (run, _, node, ids) = reliability_fixture(&store, 2).await;
    let tracked = unique_id("tracked");
    let lost = unique_id("lost");
    store
        .claim_batch(run, &node, &tracked)
        .await
        .unwrap()
        .unwrap();
    store.claim_batch(run, &node, &lost).await.unwrap().unwrap();
    sqlx::query("UPDATE batches SET claimed_at=now()-interval '2 hours' WHERE run_id=$1")
        .bind(run)
        .execute(store.pool())
        .await
        .unwrap();
    assert_eq!(
        store
            .release_untracked_claims(run, &node, std::slice::from_ref(&tracked))
            .await
            .unwrap(),
        1
    );
    let states: Vec<(i64, String)> =
        sqlx::query_as("SELECT id,status FROM batches WHERE run_id=$1 ORDER BY id")
            .bind(run)
            .fetch_all(store.pool())
            .await
            .unwrap();
    assert_eq!(
        states,
        vec![(ids[0], "claimed".into()), (ids[1], "pending".into())]
    );
    assert_eq!(
        store.reclaim_abandoned_batches(run).await.unwrap(),
        0,
        "live, slow tracked work must not be reclaimed"
    );
    assert!(
        store
            .submit_batch_results(ids[1], &node, &lost, &empty_batch_result(), 1.0)
            .await
            .unwrap_err()
            .is_batch_ownership_lost()
    );
    store
        .submit_batch_results(ids[0], &node, &tracked, &empty_batch_result(), 1.0)
        .await
        .unwrap();
    sqlx::query("DELETE FROM nodes WHERE uuid=$1")
        .bind(node)
        .execute(store.pool())
        .await
        .unwrap();
    store.remove_run(run).await.unwrap();
}

struct CountingEvaluator(std::sync::Arc<std::sync::atomic::AtomicUsize>);
impl gammaboard::evaluation::Evaluator for CountingEvaluator {
    fn get_domain(&self) -> gammaboard::Domain {
        gammaboard::Domain::rectangular(1, 0)
    }
    fn eval_batch(
        &mut self,
        _batch: &Batch,
        _accumulator: &gammaboard::core::AccumulatorConfig,
        _options: gammaboard::evaluation::EvalBatchOptions,
    ) -> Result<gammaboard::evaluation::BatchResult, gammaboard::core::EvalError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(empty_batch_result())
    }
}

async fn fault_test_runner(
    run: i32,
    node: &str,
) -> (
    gammaboard::runners::EvaluatorRunner<PgStore>,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    let url = std::env::var("GAMMABOARD_TEST_DATABASE_URL").unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|conn, _| {
            Box::pin(async move {
                sqlx::query("SET statement_timeout='100ms'")
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let config = gammaboard::core::EvaluatorConfig::Unit {
        params: Default::default(),
    };
    let runner = gammaboard::runners::EvaluatorRunner::new(
        PgStore::new(pool),
        run,
        node,
        node,
        config,
        Box::new(CountingEvaluator(calls.clone())),
        gammaboard::Domain::rectangular(1, 0),
        gammaboard::runners::EvaluatorRunnerParams {
            performance_snapshot_interval_ms: 60000,
        },
        3,
    );
    (runner, calls)
}

async fn drain_reliability_runner(
    store: &PgStore,
    run: i32,
    runner: &mut gammaboard::runners::EvaluatorRunner<PgStore>,
    expected: i64,
) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            // A submission that timed out before the lock was released can
            // still deliver that error on the first post-recovery tick.
            if let Err(error) = runner.tick().await {
                assert!(error.to_string().contains("statement timeout"), "{error}");
            }
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM batches WHERE run_id=$1 AND status='completed'",
            )
            .bind(run)
            .fetch_one(store.pool())
            .await
            .unwrap();
            if count == expected {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
        runner.stop().await.unwrap();
    })
    .await
    .expect("evaluator should recover without reassignment");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn evaluator_retains_claim_after_task_context_database_timeout() {
    let (_guard, store) = locked_test_store().await;
    let (run, _, node, _) = reliability_fixture(&store, 3).await;
    let (mut runner, calls) = fault_test_runner(run, &node).await;
    let mut lock = store.pool().begin().await.unwrap();
    sqlx::query("LOCK TABLE run_tasks IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let error = runner.tick().await.unwrap_err();
    assert!(error.to_string().contains("statement timeout"), "{error}");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    lock.rollback().await.unwrap();
    drain_reliability_runner(&store, run, &mut runner, 3).await;
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    sqlx::query("DELETE FROM nodes WHERE uuid=$1")
        .bind(node)
        .execute(store.pool())
        .await
        .unwrap();
    store.remove_run(run).await.unwrap();
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn evaluator_retains_both_results_when_submission_database_times_out() {
    let (_guard, store) = locked_test_store().await;
    let (run, _, node, _) = reliability_fixture(&store, 3).await;
    let (mut runner, calls) = fault_test_runner(run, &node).await;
    let mut lock = store.pool().begin().await.unwrap();
    sqlx::query("LOCK TABLE batch_results IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    runner.tick().await.unwrap(); // Computes first result and starts its asynchronous submit.
    let mut errors = 0;
    for _ in 0..6 {
        if let Err(error) = runner.tick().await {
            assert!(error.to_string().contains("statement timeout"), "{error}");
            errors += 1;
        }
    }
    assert!(errors > 0);
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the next computed result stays buffered while the first submission retries"
    );
    lock.rollback().await.unwrap();
    drain_reliability_runner(&store, run, &mut runner, 3).await;
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "database retries must not reevaluate samples"
    );
    sqlx::query("DELETE FROM nodes WHERE uuid=$1")
        .bind(node)
        .execute(store.pool())
        .await
        .unwrap();
    store.remove_run(run).await.unwrap();
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn launch_connection_budget_rejects_before_creating_workers() {
    let (_guard, store) = locked_test_store().await;
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM nodes")
        .fetch_one(store.pool())
        .await
        .unwrap();
    let error = store
        .reserve_worker_launch(
            "local",
            vec![serde_json::json!({"count":1000000,"name_prefix":"over-budget"})],
        )
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("worker connection budget exceeded"),
        "{error}"
    );
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM nodes")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(before, after);
}
