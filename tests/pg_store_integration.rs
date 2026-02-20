use gammaboard::core::{AssignmentLeaseStore, WorkQueueStore};
use gammaboard::{Batch, PgStore};
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
    let worker_id = unique_id("test-worker");

    let run_id: i32 =
        sqlx::query_scalar("INSERT INTO runs (status) VALUES ('running') RETURNING id")
            .fetch_one(store.pool())
            .await
            .expect("insert run");

    sqlx::query(
        r#"
        INSERT INTO workers (
            worker_id, node_id, role, implementation, version, node_specs, status
        ) VALUES (
            $1, NULL, 'evaluator', 'test_impl', 'v1', '{}'::jsonb, 'active'
        )
        "#,
    )
    .bind(&worker_id)
    .execute(store.pool())
    .await
    .expect("insert worker");

    store
        .assign_evaluator(run_id, &worker_id)
        .await
        .expect("assign evaluator");

    let batch = Batch::from_flat_data(1, 1, 0, vec![1.0], vec![]).expect("batch");
    store
        .insert_batch(run_id, &batch)
        .await
        .expect("insert batch");

    let claimed = store
        .claim_batch(run_id, &worker_id)
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
    sqlx::query("DELETE FROM workers WHERE worker_id = $1")
        .bind(&worker_id)
        .execute(store.pool())
        .await
        .expect("cleanup worker");
}

#[tokio::test]
#[ignore = "requires postgres with project migrations applied"]
async fn claim_batch_rejects_unassigned_or_inactive_assignment() {
    let Some(store) = test_store().await else {
        return;
    };
    let worker_id = unique_id("test-worker");

    let run_id: i32 =
        sqlx::query_scalar("INSERT INTO runs (status) VALUES ('running') RETURNING id")
            .fetch_one(store.pool())
            .await
            .expect("insert run");

    sqlx::query(
        r#"
        INSERT INTO workers (
            worker_id, node_id, role, implementation, version, node_specs, status
        ) VALUES (
            $1, NULL, 'evaluator', 'test_impl', 'v1', '{}'::jsonb, 'active'
        )
        "#,
    )
    .bind(&worker_id)
    .execute(store.pool())
    .await
    .expect("insert worker");

    let batch = Batch::from_flat_data(1, 1, 0, vec![2.0], vec![]).expect("batch");
    store
        .insert_batch(run_id, &batch)
        .await
        .expect("insert batch");

    let unassigned_claim = store
        .claim_batch(run_id, &worker_id)
        .await
        .expect("claim batch while unassigned");
    assert!(
        unassigned_claim.is_none(),
        "unassigned evaluator should not be able to claim"
    );

    store
        .assign_evaluator(run_id, &worker_id)
        .await
        .expect("assign evaluator");
    store
        .unassign_evaluator(run_id, &worker_id)
        .await
        .expect("unassign evaluator");

    let inactive_claim = store
        .claim_batch(run_id, &worker_id)
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
    sqlx::query("DELETE FROM workers WHERE worker_id = $1")
        .bind(&worker_id)
        .execute(store.pool())
        .await
        .expect("cleanup worker");
}
