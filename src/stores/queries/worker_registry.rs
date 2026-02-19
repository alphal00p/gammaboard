use crate::contracts::{Worker, WorkerStatus};
use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use sqlx::PgPool;

pub(crate) struct WorkerRaw {
    pub worker_id: String,
    pub node_id: Option<String>,
    pub role: String,
    pub implementation: String,
    pub version: String,
    pub node_specs: JsonValue,
    pub status: String,
    pub last_seen: Option<DateTime<Utc>>,
}

pub(crate) async fn register_worker(pool: &PgPool, worker: &Worker) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO workers (
            worker_id,
            node_id,
            role,
            implementation,
            version,
            node_specs,
            status,
            last_seen
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, now())
        ON CONFLICT (worker_id) DO UPDATE
        SET
            node_id = EXCLUDED.node_id,
            role = EXCLUDED.role,
            implementation = EXCLUDED.implementation,
            version = EXCLUDED.version,
            node_specs = EXCLUDED.node_specs,
            status = EXCLUDED.status,
            last_seen = now(),
            updated_at = now()
        "#,
    )
    .bind(&worker.worker_id)
    .bind(&worker.node_id)
    .bind(worker.role.as_str())
    .bind(&worker.implementation)
    .bind(&worker.version)
    .bind(&worker.node_specs)
    .bind(worker.status.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn heartbeat_worker(pool: &PgPool, worker_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE workers
        SET last_seen = now(), updated_at = now()
        WHERE worker_id = $1
        "#,
    )
    .bind(worker_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn update_worker_status(
    pool: &PgPool,
    worker_id: &str,
    worker_status: WorkerStatus,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE workers
        SET status = $2, updated_at = now()
        WHERE worker_id = $1
        "#,
    )
    .bind(worker_id)
    .bind(worker_status.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn get_worker(
    pool: &PgPool,
    worker_id: &str,
) -> Result<Option<WorkerRaw>, sqlx::Error> {
    type WorkerRow = (
        String,
        Option<String>,
        String,
        String,
        String,
        JsonValue,
        String,
        Option<DateTime<Utc>>,
    );

    let row = sqlx::query_as::<_, WorkerRow>(
        r#"
        SELECT
            worker_id,
            node_id,
            role,
            implementation,
            version,
            node_specs,
            status,
            last_seen
        FROM workers
        WHERE worker_id = $1
        "#,
    )
    .bind(worker_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(
        |(worker_id, node_id, role, implementation, version, node_specs, status, last_seen)| {
            WorkerRaw {
                worker_id,
                node_id,
                role,
                implementation,
                version,
                node_specs,
                status,
                last_seen,
            }
        },
    ))
}
