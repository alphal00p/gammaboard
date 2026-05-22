use gammaboard::config::RuntimeConfig;
use gammaboard::core::{
    AccumulatorMetricName, BatchFailOutcome, ControlPlaneStore, RunTaskInput, RunTaskSpec,
    RunTaskStore, SampleStopCondition, StoreError, TaskMeasurementOutput, WorkQueueStore,
    WorkerRole, next_batch_ids,
};
use gammaboard::{Batch, LatentBatchSpec, MeasurementResult, PgStore, Point};
use sqlx::postgres::PgPoolOptions;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_nanos();
    format!("{prefix}-{nanos}")
}

async fn test_store() -> Option<PgStore> {
    let db_url = RuntimeConfig::load("ops/local/config/runtime.toml")
        .ok()?
        .database
        .url;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url)
        .await
        .ok()?;
    Some(PgStore::new(pool))
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn claim_batch_requires_active_assignment() {
    let Some(store) = test_store().await else {
        return;
    };
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

    let task_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO run_tasks (run_id, name, sequence_nr, task, state)
        VALUES ($1, 'sample-0', 0, '{"kind":"pause"}'::jsonb, 'completed')
        RETURNING id
        "#,
    )
    .bind(run_id)
    .fetch_one(store.pool())
    .await
    .expect("insert run task");

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

    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(run_id)
        .execute(store.pool())
        .await
        .expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn task_measurement_output_round_trips() {
    let Some(store) = test_store().await else {
        return;
    };

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
                    completed_samples_per_second: Some(100.0),
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

    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(run_id)
        .execute(store.pool())
        .await
        .expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn claim_batch_rejects_unassigned_or_inactive_assignment() {
    let Some(store) = test_store().await else {
        return;
    };
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

    let task_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO run_tasks (run_id, name, sequence_nr, task, state)
        VALUES ($1, 'sample-0', 0, '{"kind":"pause"}'::jsonb, 'completed')
        RETURNING id
        "#,
    )
    .bind(run_id)
    .fetch_one(store.pool())
    .await
    .expect("insert run task");

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

    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(run_id)
        .execute(store.pool())
        .await
        .expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn claim_batch_claims_exactly_one_pending_batch() {
    let Some(store) = test_store().await else {
        return;
    };
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

    let task_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO run_tasks (run_id, name, sequence_nr, task, state)
        VALUES ($1, 'sample-0', 0, '{"kind":"pause"}'::jsonb, 'completed')
        RETURNING id
        "#,
    )
    .bind(run_id)
    .fetch_one(store.pool())
    .await
    .expect("insert run task");

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

    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(run_id)
        .execute(store.pool())
        .await
        .expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn sampler_aggregator_desired_assignment_is_unique_per_run() {
    let Some(store) = test_store().await else {
        return;
    };
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

    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(run_id)
        .execute(store.pool())
        .await
        .expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn cleanup_consumed_completed_batches_does_not_remove_failed_batches() {
    let Some(store) = test_store().await else {
        return;
    };

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

    let task_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO run_tasks (run_id, name, sequence_nr, task, state)
        VALUES ($1, 'sample-0', 0, '{"kind":"pause"}'::jsonb, 'completed')
        RETURNING id
        "#,
    )
    .bind(run_id)
    .fetch_one(store.pool())
    .await
    .expect("insert run task");

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

    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(run_id)
        .execute(store.pool())
        .await
        .expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn expired_sampler_assignment_does_not_block_new_sampler_assignment() {
    let Some(store) = test_store().await else {
        return;
    };
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

    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(run_id)
        .execute(store.pool())
        .await
        .expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn assigning_new_role_replaces_existing_desired_assignment_for_node() {
    let Some(store) = test_store().await else {
        return;
    };
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

    sqlx::query("DELETE FROM runs WHERE id = $1 OR id = $2")
        .bind(run_a)
        .bind(run_b)
        .execute(store.pool())
        .await
        .expect("cleanup runs");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn assigning_dead_node_returns_not_found() {
    let Some(store) = test_store().await else {
        return;
    };
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

    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(run_id)
        .execute(store.pool())
        .await
        .expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn expiring_node_lease_clears_desired_assignment() {
    let Some(store) = test_store().await else {
        return;
    };
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

    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(run_id)
        .execute(store.pool())
        .await
        .expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn shutdown_request_clears_desired_assignment_but_keeps_current_assignment() {
    let Some(store) = test_store().await else {
        return;
    };
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

    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(run_id)
        .execute(store.pool())
        .await
        .expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn expired_shutdown_request_does_not_affect_replacement_node() {
    let Some(store) = test_store().await else {
        return;
    };
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
    let Some(store) = test_store().await else {
        return;
    };
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

    let rows_updated = store
        .request_all_nodes_shutdown()
        .await
        .expect("request all node shutdown");
    assert_eq!(rows_updated, 2);

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

    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(run_id)
        .execute(store.pool())
        .await
        .expect("cleanup run");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn sampler_aggregator_current_assignment_is_unique_per_run() {
    let Some(store) = test_store().await else {
        return;
    };
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

    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(run_id)
        .execute(store.pool())
        .await
        .expect("cleanup run");
}
