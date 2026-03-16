use gammaboard::core::{ControlPlaneStore, StoreError, WorkQueueStore, WorkerRole};
use gammaboard::{Batch, LatentBatchSpec, PgStore};
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
    let db_url = std::env::var("DATABASE_URL").ok()?;
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
    let node_id = unique_id("node");

    let run_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (
            name,
            integration_params,
            point_spec
        ) VALUES (
            'claim-batch-active',
            '{}'::jsonb,
            '{"continuous_dims":1,"discrete_dims":0}'::jsonb
        )
        RETURNING id
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("insert run");

    store.register_node(&node_id).await.expect("register node");
    store
        .set_current_assignment(&node_id, WorkerRole::Evaluator, run_id)
        .await
        .expect("set current evaluator assignment");

    let batch = Batch::from_flat_data(1, 1, 0, vec![1.0], vec![]).expect("batch");
    let latent_batch = LatentBatchSpec::from_batch(&batch).with_version(1);
    store
        .insert_batch(run_id, &latent_batch, true)
        .await
        .expect("insert batch");

    let claimed = store
        .claim_batch(run_id, &node_id)
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
async fn claim_batch_rejects_unassigned_or_inactive_assignment() {
    let Some(store) = test_store().await else {
        return;
    };
    let node_id = unique_id("node");

    let run_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (
            name,
            integration_params,
            point_spec
        ) VALUES (
            'claim-batch-inactive',
            '{}'::jsonb,
            '{"continuous_dims":1,"discrete_dims":0}'::jsonb
        )
        RETURNING id
        "#,
    )
    .fetch_one(store.pool())
    .await
    .expect("insert run");

    store.register_node(&node_id).await.expect("register node");

    let batch = Batch::from_flat_data(1, 1, 0, vec![2.0], vec![]).expect("batch");
    let latent_batch = LatentBatchSpec::from_batch(&batch).with_version(1);
    store
        .insert_batch(run_id, &latent_batch, true)
        .await
        .expect("insert batch");

    let unassigned_claim = store
        .claim_batch(run_id, &node_id)
        .await
        .expect("claim batch while unassigned");
    assert!(
        unassigned_claim.is_none(),
        "unassigned evaluator should not be able to claim"
    );

    store
        .set_current_assignment(&node_id, WorkerRole::Evaluator, run_id)
        .await
        .expect("set current evaluator assignment");
    store
        .clear_current_assignment(&node_id)
        .await
        .expect("clear current assignment");

    let inactive_claim = store
        .claim_batch(run_id, &node_id)
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
async fn sampler_aggregator_desired_assignment_is_unique_per_run() {
    let Some(store) = test_store().await else {
        return;
    };
    let node_a = unique_id("node-a");
    let node_b = unique_id("node-b");

    let run_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO runs (
            name,
            integration_params,
            point_spec
        ) VALUES (
            'test-run',
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
async fn assigning_new_role_replaces_existing_desired_assignment_for_node() {
    let Some(store) = test_store().await else {
        return;
    };
    let node_id = unique_id("node");

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
        .upsert_desired_assignment(&node_id, WorkerRole::Evaluator, run_a)
        .await
        .expect("assign evaluator");
    store
        .upsert_desired_assignment(&node_id, WorkerRole::SamplerAggregator, run_b)
        .await
        .expect("replace desired assignment");

    let assignment = store
        .get_desired_assignment(&node_id)
        .await
        .expect("load desired assignment")
        .expect("assignment should exist");
    assert_eq!(assignment.role, WorkerRole::SamplerAggregator);
    assert_eq!(assignment.run_id, run_b);

    let desired_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM nodes
        WHERE node_id = $1
          AND desired_run_id IS NOT NULL
          AND desired_role IS NOT NULL
        "#,
    )
    .bind(&node_id)
    .fetch_one(store.pool())
    .await
    .expect("count desired assignments");
    assert_eq!(
        desired_count, 1,
        "node should have exactly one desired assignment"
    );

    store
        .clear_desired_assignment(&node_id)
        .await
        .expect("clear desired assignment");
    assert!(
        store
            .get_desired_assignment(&node_id)
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
async fn sampler_aggregator_current_assignment_is_unique_per_run() {
    let Some(store) = test_store().await else {
        return;
    };
    let node_a = unique_id("node-a");
    let node_b = unique_id("node-b");

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

    store.register_node(&node_a).await.expect("register node a");
    store.register_node(&node_b).await.expect("register node b");

    store
        .set_current_assignment(&node_a, WorkerRole::SamplerAggregator, run_id)
        .await
        .expect("set current sampler on node a");

    let err = store
        .set_current_assignment(&node_b, WorkerRole::SamplerAggregator, run_id)
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
