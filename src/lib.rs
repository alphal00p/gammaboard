//! Gammaboard - Adaptive Numerical Integration System
//!
//! This library provides database abstractions for distributed adaptive
//! numerical integration using PostgreSQL as a work queue.

pub mod batch;

use dotenvy::dotenv;
use sqlx::{postgres::PgPoolOptions, PgPool, Pool, Postgres};
use std::env;

pub use batch::{Batch, BatchRecord, BatchResults, BatchStatus, WeightedPoint};

/// Create a PostgreSQL connection pool
///
/// Loads DATABASE_URL from environment (via .env file) and creates
/// a connection pool with the specified maximum number of connections.
///
/// # Arguments
/// * `max_connections` - Maximum number of concurrent database connections
///
/// # Example
/// ```no_run
/// use gammaboard::get_pg_pool;
///
/// #[tokio::main]
/// async fn main() -> Result<(), sqlx::Error> {
///     let pool = get_pg_pool(10).await?;
///     Ok(())
/// }
/// ```
pub async fn get_pg_pool(max_connections: u32) -> Result<Pool<Postgres>, sqlx::Error> {
    dotenv().ok();
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(&db_url)
        .await
    }

    /// Type alias for database pool
    pub type DbPool = PgPool;

    // ============================================================================
    // Database Query Functions for API
    // ============================================================================

    use serde::{Deserialize, Serialize};
    use sqlx::Row;

    /// Run progress information
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct RunProgress {
        pub run_id: i32,
        pub run_status: String,
        pub started_at: Option<chrono::DateTime<chrono::Utc>>,
        pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
        pub total_batches_planned: Option<i32>,
        pub batches_completed: i32,
        pub final_result: Option<f64>,
        pub error_estimate: Option<f64>,
        pub total_batches: i64,
        pub total_samples: i64,
        pub pending_batches: i64,
        pub claimed_batches: i64,
        pub completed_batches: i64,
        pub failed_batches: i64,
        pub completion_rate: f64,
    }

    /// Work queue statistics
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct WorkQueueStats {
        pub run_id: i32,
        pub status: String,
        pub batch_count: i64,
        pub total_samples: i64,
        pub avg_batch_time_ms: Option<f64>,
        pub avg_sample_time_ms: Option<f64>,
    }

    /// Completed batch data
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct CompletedBatch {
        pub id: i64,
        pub batch_size: i32,
        pub points: serde_json::Value,
        pub results: Option<serde_json::Value>,
        pub total_eval_time_ms: Option<f64>,
        pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    }

    /// Flattened sample data for visualization
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SampleData {
        pub point: serde_json::Value,
        pub weight: f64,
        pub value: f64,
    }

    /// Get all runs with their progress
    pub async fn get_all_runs(pool: &PgPool) -> Result<Vec<RunProgress>, sqlx::Error> {
        let rows = sqlx::query(
            r#"
            SELECT
                run_id,
                run_status,
                started_at,
                completed_at,
                total_batches_planned,
                batches_completed,
                final_result,
                error_estimate,
                total_batches,
                total_samples,
                pending_batches,
                claimed_batches,
                completed_batches,
                failed_batches,
                completion_rate
            FROM run_progress
            ORDER BY started_at DESC
            "#,
        )
        .fetch_all(pool)
        .await?;

        let mut runs = Vec::new();
        for row in rows {
            runs.push(RunProgress {
                run_id: row.get("run_id"),
                run_status: row.get("run_status"),
                started_at: row.get("started_at"),
                completed_at: row.get("completed_at"),
                total_batches_planned: row.get("total_batches_planned"),
                batches_completed: row.get("batches_completed"),
                final_result: row.get("final_result"),
                error_estimate: row.get("error_estimate"),
                total_batches: row.get("total_batches"),
                total_samples: row.get("total_samples"),
                pending_batches: row.get("pending_batches"),
                claimed_batches: row.get("claimed_batches"),
                completed_batches: row.get("completed_batches"),
                failed_batches: row.get("failed_batches"),
                completion_rate: row.get("completion_rate"),
            });
        }

        Ok(runs)
    }

    /// Get specific run progress
    pub async fn get_run_progress(pool: &PgPool, run_id: i32) -> Result<Option<RunProgress>, sqlx::Error> {
        let row = sqlx::query(
            r#"
            SELECT
                run_id,
                run_status,
                started_at,
                completed_at,
                total_batches_planned,
                batches_completed,
                final_result,
                error_estimate,
                total_batches,
                total_samples,
                pending_batches,
                claimed_batches,
                completed_batches,
                failed_batches,
                completion_rate
            FROM run_progress
            WHERE run_id = $1
            "#,
        )
        .bind(run_id)
        .fetch_optional(pool)
        .await?;

        Ok(row.map(|r| RunProgress {
            run_id: r.get("run_id"),
            run_status: r.get("run_status"),
            started_at: r.get("started_at"),
            completed_at: r.get("completed_at"),
            total_batches_planned: r.get("total_batches_planned"),
            batches_completed: r.get("batches_completed"),
            final_result: r.get("final_result"),
            error_estimate: r.get("error_estimate"),
            total_batches: r.get("total_batches"),
            total_samples: r.get("total_samples"),
            pending_batches: r.get("pending_batches"),
            claimed_batches: r.get("claimed_batches"),
            completed_batches: r.get("completed_batches"),
            failed_batches: r.get("failed_batches"),
            completion_rate: r.get("completion_rate"),
        }))
    }

    /// Get work queue statistics for a run
    pub async fn get_work_queue_stats(pool: &PgPool, run_id: i32) -> Result<Vec<WorkQueueStats>, sqlx::Error> {
        let rows = sqlx::query(
            r#"
            SELECT
                run_id,
                status,
                batch_count,
                total_samples,
                avg_batch_time_ms,
                avg_sample_time_ms
            FROM work_queue_stats
            WHERE run_id = $1
            "#,
        )
        .bind(run_id)
        .fetch_all(pool)
        .await?;

        let mut stats = Vec::new();
        for row in rows {
            stats.push(WorkQueueStats {
                run_id: row.get("run_id"),
                status: row.get("status"),
                batch_count: row.get("batch_count"),
                total_samples: row.get("total_samples"),
                avg_batch_time_ms: row.get("avg_batch_time_ms"),
                avg_sample_time_ms: row.get("avg_sample_time_ms"),
            });
        }

        Ok(stats)
    }

    /// Get completed batches for a run
    pub async fn get_completed_batches(
        pool: &PgPool,
        run_id: i32,
        limit: i64,
    ) -> Result<Vec<CompletedBatch>, sqlx::Error> {
        let rows = sqlx::query(
            r#"
            SELECT
                id,
                batch_size,
                points,
                results,
                total_eval_time_ms,
                completed_at
            FROM batches
            WHERE run_id = $1 AND status = 'completed'
            ORDER BY completed_at DESC
            LIMIT $2
            "#,
        )
        .bind(run_id)
        .bind(limit)
        .fetch_all(pool)
        .await?;

        let mut batches = Vec::new();
        for row in rows {
            batches.push(CompletedBatch {
                id: row.get("id"),
                batch_size: row.get("batch_size"),
                points: row.get("points"),
                results: row.get("results"),
                total_eval_time_ms: row.get("total_eval_time_ms"),
                completed_at: row.get("completed_at"),
            });
        }

        Ok(batches)
    }

    /// Get flattened sample data for visualization
    pub async fn get_sample_data(
        pool: &PgPool,
        run_id: i32,
        limit: i64,
    ) -> Result<Vec<SampleData>, sqlx::Error> {
        let batches = get_completed_batches(pool, run_id, 50).await?;

        let mut samples = Vec::new();
        for batch in batches {
            // Parse batch data
            if let Some(batch_data) = Batch::from_json(&batch.points).ok() {
                if let Some(results_json) = batch.results {
                    if let Some(results) = BatchResults::from_json(&results_json).ok() {
                        // Flatten into individual samples
                        for (i, point) in batch_data.points.iter().enumerate() {
                            if i < results.values.len() {
                                samples.push(SampleData {
                                    point: point.point.clone(),
                                    weight: point.weight,
                                    value: results.values[i],
                                });

                                if samples.len() >= limit as usize {
                                    return Ok(samples);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(samples)
    }
