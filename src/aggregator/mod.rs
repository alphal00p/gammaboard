//! Sampler-aggregator library logic.
//!
//! This module contains pure aggregation logic plus orchestration that delegates
//! *all* database operations to `queries` wrappers. Binaries should only be
//! responsible for scheduling and process lifecycle.

use crate::{Batch, BatchResults, queries};
use serde_json::Value as JsonValue;
use sqlx::PgPool;

#[derive(Debug, Clone)]
pub struct AggregationSnapshot {
    pub nr_samples: i64,
    pub nr_batches: i32,
    pub sum: f64,
    pub sum_x2: f64,
    pub sum_abs: f64,
    pub max: Option<f64>,
    pub min: Option<f64>,
    pub weighted_sum: f64,
    pub weighted_sum_x2: f64,
    pub sum_weights: f64,
    pub mean: Option<f64>,
    pub variance: Option<f64>,
    pub std_dev: Option<f64>,
    pub error_estimate: Option<f64>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct DeltaAggregation {
    pub nr_samples: i64,
    pub nr_batches: i32,
    pub sum: f64,
    pub sum_x2: f64,
    pub sum_abs: f64,
    pub max: Option<f64>,
    pub min: Option<f64>,
    pub weighted_sum: f64,
    pub weighted_sum_x2: f64,
    pub sum_weights: f64,
}

impl DeltaAggregation {
    pub fn is_empty(&self) -> bool {
        self.nr_samples == 0
    }
}

/// Aggregate a single run:
/// - loads latest snapshot (if any)
/// - reads completed batches since that snapshot
/// - aggregates them into a delta
/// - inserts a new `aggregated_results` snapshot
/// - updates `runs` summary fields
pub async fn aggregate_run(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<AggregationSnapshot>, sqlx::Error> {
    let last_snapshot = fetch_latest_snapshot(pool, run_id).await?;
    let since = last_snapshot.as_ref().map(|s| s.created_at);

    let batches = queries::get_completed_batches_points_results_since(pool, run_id, since).await?;
    if batches.is_empty() {
        return Ok(None);
    }

    let delta = aggregate_batches(&batches);
    if delta.is_empty() {
        return Ok(None);
    }

    let combined = combine_snapshot(last_snapshot, &delta);

    queries::insert_aggregated_results_snapshot(
        pool,
        run_id,
        combined.nr_samples,
        combined.nr_batches,
        combined.sum,
        combined.sum_x2,
        combined.sum_abs,
        combined.max,
        combined.min,
        combined.weighted_sum,
        combined.weighted_sum_x2,
        combined.sum_weights,
        combined.mean,
        combined.variance,
        combined.std_dev,
        combined.error_estimate,
    )
    .await?;

    let final_result = if combined.sum_weights > 0.0 {
        Some(combined.weighted_sum / combined.sum_weights)
    } else {
        combined.mean
    };

    queries::update_run_summary_from_snapshot(
        pool,
        run_id,
        delta.nr_batches,
        final_result,
        combined.error_estimate,
    )
    .await?;

    Ok(Some(combined))
}

/// Fetch latest snapshot for a run (or None) using queries wrappers.
pub async fn fetch_latest_snapshot(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<AggregationSnapshot>, sqlx::Error> {
    let row = queries::get_latest_aggregation_snapshot(pool, run_id).await?;

    Ok(row.map(
        |(
            nr_samples,
            nr_batches,
            sum,
            sum_x2,
            sum_abs,
            max,
            min,
            weighted_sum,
            weighted_sum_x2,
            sum_weights,
            mean,
            variance,
            std_dev,
            error_estimate,
            created_at,
        )| AggregationSnapshot {
            nr_samples,
            nr_batches,
            sum,
            sum_x2,
            sum_abs,
            max,
            min,
            weighted_sum,
            weighted_sum_x2,
            sum_weights,
            mean,
            variance,
            std_dev,
            error_estimate,
            created_at,
        },
    ))
}

/// Pure aggregation over a set of completed batches.
pub fn aggregate_batches(batches: &[(JsonValue, JsonValue)]) -> DeltaAggregation {
    let mut delta = DeltaAggregation {
        nr_samples: 0,
        nr_batches: 0,
        sum: 0.0,
        sum_x2: 0.0,
        sum_abs: 0.0,
        max: None,
        min: None,
        weighted_sum: 0.0,
        weighted_sum_x2: 0.0,
        sum_weights: 0.0,
    };

    for (points_json, results_json) in batches {
        let batch = match Batch::from_json(points_json) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let results = match BatchResults::from_json(results_json) {
            Ok(r) => r,
            Err(_) => continue,
        };

        if results.values.len() != batch.points.len() {
            continue;
        }

        delta.nr_batches += 1;

        for (point, value) in batch.points.iter().zip(results.values.iter()) {
            let v = *value;
            let w = point.weight;

            delta.nr_samples += 1;
            delta.sum += v;
            delta.sum_x2 += v * v;
            delta.sum_abs += v.abs();

            delta.weighted_sum += v * w;
            delta.weighted_sum_x2 += (v * w) * (v * w);
            delta.sum_weights += w;

            delta.max = Some(delta.max.map_or(v, |m| m.max(v)));
            delta.min = Some(delta.min.map_or(v, |m| m.min(v)));
        }
    }

    delta
}

/// Combine previous snapshot + delta into a new snapshot.
pub fn combine_snapshot(
    previous: Option<AggregationSnapshot>,
    delta: &DeltaAggregation,
) -> AggregationSnapshot {
    let (
        mut nr_samples,
        mut nr_batches,
        mut sum,
        mut sum_x2,
        mut sum_abs,
        mut max,
        mut min,
        mut weighted_sum,
        mut weighted_sum_x2,
        mut sum_weights,
    ) = if let Some(prev) = previous {
        (
            prev.nr_samples,
            prev.nr_batches,
            prev.sum,
            prev.sum_x2,
            prev.sum_abs,
            prev.max,
            prev.min,
            prev.weighted_sum,
            prev.weighted_sum_x2,
            prev.sum_weights,
        )
    } else {
        (0, 0, 0.0, 0.0, 0.0, None, None, 0.0, 0.0, 0.0)
    };

    nr_samples += delta.nr_samples;
    nr_batches += delta.nr_batches;
    sum += delta.sum;
    sum_x2 += delta.sum_x2;
    sum_abs += delta.sum_abs;
    weighted_sum += delta.weighted_sum;
    weighted_sum_x2 += delta.weighted_sum_x2;
    sum_weights += delta.sum_weights;

    max = match (max, delta.max) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (None, Some(b)) => Some(b),
        (Some(a), None) => Some(a),
        (None, None) => None,
    };

    min = match (min, delta.min) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (None, Some(b)) => Some(b),
        (Some(a), None) => Some(a),
        (None, None) => None,
    };

    let mean = if nr_samples > 0 {
        Some(sum / nr_samples as f64)
    } else {
        None
    };

    let variance = if nr_samples > 1 {
        let mu = mean.unwrap_or(0.0);
        let var = (sum_x2 / nr_samples as f64) - (mu * mu);
        Some(var.max(0.0))
    } else {
        None
    };

    let std_dev = variance.map(|v| v.sqrt());
    let error_estimate = if let (Some(sd), true) = (std_dev, nr_samples > 0) {
        Some(sd / (nr_samples as f64).sqrt())
    } else {
        None
    };

    AggregationSnapshot {
        nr_samples,
        nr_batches,
        sum,
        sum_x2,
        sum_abs,
        max,
        min,
        weighted_sum,
        weighted_sum_x2,
        sum_weights,
        mean,
        variance,
        std_dev,
        error_estimate,
        created_at: chrono::Utc::now(),
    }
}

/// Convenience helper: list run ids (used by scheduler binaries).
pub async fn list_run_ids(pool: &PgPool) -> Result<Vec<i32>, sqlx::Error> {
    queries::list_run_ids(pool).await
}
