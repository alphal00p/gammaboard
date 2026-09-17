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
    })
    .await
    .expect("node metadata must not wait for the task lock");
    task_writer.rollback().await.expect("release task writer");
    sleep(Duration::from_millis(50)).await;
    store
        .announce_node(&node_name, &node_uuid, &capabilities)
        .await
        .expect("account heartbeat");
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
        .claim_batch(run_id, &node_uuid)
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
        .claim_batch(run_id, &node_uuid)
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
        .claim_batch(run_id, &node_uuid)
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
        .claim_batch(run_id, &node_uuid)
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
        .upsert_desired_assignment(&node_a, WorkerRole::SamplerAggregator, run_id)
        .await
        .expect("assign first sampler");

    let err = store
        .upsert_desired_assignment(&node_b, WorkerRole::SamplerAggregator, run_id)
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

    let outcome = store
        .fail_batch(batch_ids[0], "forced failure", 1)
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
        .upsert_desired_assignment(&stale_node, WorkerRole::SamplerAggregator, run_id)
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
        .upsert_desired_assignment(&fresh_node, WorkerRole::SamplerAggregator, run_id)
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
        .upsert_desired_assignment(&node_name, WorkerRole::Evaluator, run_a)
        .await
        .expect("assign evaluator");
    store
        .upsert_desired_assignment(&node_name, WorkerRole::SamplerAggregator, run_b)
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
        .upsert_desired_assignment(&node_name, WorkerRole::Evaluator, run_id)
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
        .upsert_desired_assignment(&node_name, WorkerRole::SamplerAggregator, run_id)
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
        .upsert_desired_assignment(&node_name, WorkerRole::SamplerAggregator, run_id)
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
        .upsert_desired_assignment(&node_a, WorkerRole::SamplerAggregator, run_id)
        .await
        .expect("assign desired sampler");
    store
        .upsert_desired_assignment(&node_b, WorkerRole::Evaluator, run_id)
        .await
        .expect("assign desired evaluator");

    let live_nodes: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM nodes
        WHERE lease_expires_at > now()
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("count live nodes");
    let rows_updated = store
        .request_all_nodes_shutdown()
        .await
        .expect("request all node shutdown");
    assert_eq!(rows_updated, live_nodes as u64);

    let desired_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM nodes
        WHERE name = ANY($1)
          AND desired_run_id IS NOT NULL
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
async fn prefetch_yields_to_unserved_peers_but_never_strands_work() {
    let (_guard, store) = locked_test_store().await;
    let run_id: i32 = sqlx::query_scalar(
        "INSERT INTO runs (name,integration_params,point_spec) VALUES ('fair-prefetch','{}','{\"continuous\":{\"dims\":1}}') RETURNING id"
    ).fetch_one(store.pool()).await.unwrap();
    let a = unique_id("fair-a");
    let b = unique_id("fair-b");
    for node in [&a, &b] {
        store
            .announce_node(node, node, &Default::default())
            .await
            .unwrap();
        store
            .set_current_assignment(node, WorkerRole::Evaluator, run_id)
            .await
            .unwrap();
    }
    let task_id = insert_completed_pause_task(&store, run_id).await;
    let batch = Batch::from_points([Point::new(vec![1.0], Vec::new(), 1.0)]).unwrap();
    let batches = vec![LatentBatchSpec::from_batch(&batch).build(); 5];
    store
        .insert_batches(run_id, task_id, false, &next_batch_ids(5), &batches)
        .await
        .unwrap();
    // Freeze the fresh-batch condition rather than making a wall-clock-sensitive test.
    sqlx::query("UPDATE batches SET created_at=now()+interval '1 hour' WHERE run_id=$1")
        .bind(run_id)
        .execute(store.pool())
        .await
        .unwrap();
    assert!(store.claim_batch(run_id, &a).await.unwrap().is_some());
    assert!(store.claim_batch(run_id, &a).await.unwrap().is_none());
    assert!(store.claim_batch(run_id, &b).await.unwrap().is_some());
    assert!(store.claim_batch(run_id, &a).await.unwrap().is_some());
    store
        .release_claimed_batches_for_worker(run_id, &b)
        .await
        .unwrap();
    assert!(store.claim_batch(run_id, &a).await.unwrap().is_none());
    // A live but unresponsive peer cannot indefinitely prevent prefetch.
    sqlx::query("UPDATE batches SET created_at=now()-interval '1 second' WHERE run_id=$1")
        .bind(run_id)
        .execute(store.pool())
        .await
        .unwrap();
    assert!(store.claim_batch(run_id, &a).await.unwrap().is_some());
    // Expired peers should not delay even newly inserted work.
    sqlx::query("UPDATE nodes SET lease_expires_at=now()-interval '1 second' WHERE uuid=$1")
        .bind(&b)
        .execute(store.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE batches SET created_at=now()+interval '1 hour' WHERE run_id=$1")
        .bind(run_id)
        .execute(store.pool())
        .await
        .unwrap();
    assert!(store.claim_batch(run_id, &a).await.unwrap().is_some());
    store.remove_run(run_id).await.unwrap();
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
