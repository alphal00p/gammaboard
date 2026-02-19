use sqlx::PgPool;
use std::time::Duration;

pub(crate) async fn acquire_sampler_aggregator_lease(
    pool: &PgPool,
    run_id: i32,
    worker_id: &str,
    ttl: Duration,
) -> Result<bool, sqlx::Error> {
    let ttl_secs = ttl.as_secs_f64().max(1.0);
    let row = sqlx::query(
        r#"
        INSERT INTO run_sampler_aggregator_leases (
            run_id,
            worker_id,
            lease_expires_at
        ) VALUES (
            $1,
            $2,
            now() + make_interval(secs => $3)
        )
        ON CONFLICT (run_id) DO UPDATE
        SET
            worker_id = EXCLUDED.worker_id,
            lease_expires_at = EXCLUDED.lease_expires_at,
            updated_at = now()
        WHERE
            run_sampler_aggregator_leases.worker_id = EXCLUDED.worker_id
            OR run_sampler_aggregator_leases.lease_expires_at < now()
        RETURNING run_id
        "#,
    )
    .bind(run_id)
    .bind(worker_id)
    .bind(ttl_secs)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

pub(crate) async fn renew_sampler_aggregator_lease(
    pool: &PgPool,
    run_id: i32,
    worker_id: &str,
    ttl: Duration,
) -> Result<bool, sqlx::Error> {
    let ttl_secs = ttl.as_secs_f64().max(1.0);
    let row = sqlx::query(
        r#"
        UPDATE run_sampler_aggregator_leases
        SET
            lease_expires_at = now() + make_interval(secs => $3),
            updated_at = now()
        WHERE run_id = $1 AND worker_id = $2
        RETURNING run_id
        "#,
    )
    .bind(run_id)
    .bind(worker_id)
    .bind(ttl_secs)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

pub(crate) async fn release_sampler_aggregator_lease(
    pool: &PgPool,
    run_id: i32,
    worker_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        DELETE FROM run_sampler_aggregator_leases
        WHERE run_id = $1 AND worker_id = $2
        "#,
    )
    .bind(run_id)
    .bind(worker_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn assign_evaluator(
    pool: &PgPool,
    run_id: i32,
    worker_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO run_evaluator_assignments (
            run_id,
            worker_id,
            active,
            assigned_at
        ) VALUES (
            $1, $2, true, now()
        )
        ON CONFLICT (run_id, worker_id) DO UPDATE
        SET active = true, assigned_at = now()
        "#,
    )
    .bind(run_id)
    .bind(worker_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn unassign_evaluator(
    pool: &PgPool,
    run_id: i32,
    worker_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE run_evaluator_assignments
        SET active = false
        WHERE run_id = $1 AND worker_id = $2
        "#,
    )
    .bind(run_id)
    .bind(worker_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn list_assigned_evaluators(
    pool: &PgPool,
    run_id: i32,
) -> Result<Vec<String>, sqlx::Error> {
    let rows = sqlx::query_scalar::<_, String>(
        r#"
        SELECT worker_id
        FROM run_evaluator_assignments
        WHERE run_id = $1 AND active = true
        ORDER BY assigned_at ASC
        "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}
