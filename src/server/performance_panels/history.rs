//! Whole-run history with bounded response size and viewport-dependent detail.
use super::{
    graphs::{DISPLAY_BINS, Graphs},
    *,
};
use crate::PgStore;
use crate::api::ApiError;

pub(in crate::server) async fn response(
    store: &PgStore,
    run_id: i32,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
) -> Result<Value, ApiError> {
    let bounds: (Option<DateTime<Utc>>, Option<DateTime<Utc>>) = sqlx::query_as(
        r#"
        SELECT min(first), max(last) FROM (
            SELECT min(created_at) AS first, max(created_at) AS last
            FROM evaluator_performance_history WHERE run_id=$1
            UNION ALL
            SELECT min(created_at), max(created_at)
            FROM sampler_aggregator_performance_history WHERE run_id=$1
        ) bounds
    "#,
    )
    .bind(run_id)
    .fetch_one(store.pool())
    .await
    .map_err(crate::core::StoreError::from)?;
    let Some((first, last)) = bounds.0.zip(bounds.1) else {
        return Ok(json!({"bounds": null, "panels": [], "states": []}));
    };
    let bounds = [
        first.timestamp_millis(),
        last.timestamp_millis().max(first.timestamp_millis() + 1),
    ];
    let start = start_ms.unwrap_or(bounds[0]).max(bounds[0]);
    let end = end_ms.unwrap_or(bounds[1]).min(bounds[1]);
    if start >= end {
        return Err(ApiError::BadRequest(
            "selected range does not overlap recorded history".into(),
        ));
    }
    let since = DateTime::from_timestamp_millis(start)
        .ok_or_else(|| ApiError::BadRequest("invalid start_ms".into()))?;
    // Include the last snapshot's submillisecond fraction in a full-range view.
    let until = if end == bounds[1] {
        last
    } else {
        DateTime::from_timestamp_millis(end)
            .ok_or_else(|| ApiError::BadRequest("invalid end_ms".into()))?
    };
    let mut graphs = Graphs::new([start as f64, end as f64]);
    for (evaluator, table, field) in [
        (true, "evaluator_performance_history", "metrics"),
        (
            false,
            "sampler_aggregator_performance_history",
            "runtime_metrics",
        ),
    ] {
        // Only project counters needed by graphs. Never load histogram/engine diagnostics.
        let payload = format!(
            r#"jsonb_build_object('worker_id', worker_id, 'created_at', created_at,
            '{field}', jsonb_build_object('epoch', {field}->'epoch',
            'runner_epoch', {field}->'runner_epoch', 'node_uuid', {field}->'node_uuid',
            'task_id', {field}->'task_id', 'busy', {field}->'busy',
            'samples_evaluated', {field}->'samples_evaluated',
            'completed_samples_total', {field}->'completed_samples_total'))"#
        );
        // Boundary neighbors preserve intervals when zooming below the reporting cadence.
        let query = format!(
            "SELECT payload FROM (SELECT DISTINCT ON (worker_id) created_at, {payload} AS payload FROM {table} WHERE run_id=$1 AND created_at < $2 ORDER BY worker_id, created_at DESC, id DESC) r ORDER BY created_at"
        );
        for row in sqlx::query_scalar::<_, Value>(&query)
            .bind(run_id)
            .bind(since)
            .fetch_all(store.pool())
            .await
            .map_err(crate::core::StoreError::from)?
        {
            graphs.observe(row, evaluator);
        }
        let query = format!(
            "SELECT created_at, id, {payload} FROM {table} WHERE run_id=$1 AND created_at >= $2 AND created_at <= $3 AND (created_at,id) > ($4,$5) ORDER BY created_at,id LIMIT 4096"
        );
        let mut cursor = (since, i64::MIN);
        loop {
            let page: Vec<(DateTime<Utc>, i64, Value)> = sqlx::query_as(&query)
                .bind(run_id)
                .bind(since)
                .bind(until)
                .bind(cursor.0)
                .bind(cursor.1)
                .fetch_all(store.pool())
                .await
                .map_err(crate::core::StoreError::from)?;
            let count = page.len();
            for (time, id, row) in page {
                cursor = (time, id);
                graphs.observe(row, evaluator);
            }
            if count < 4096 {
                break;
            }
        }
        let query = format!(
            "SELECT payload FROM (SELECT DISTINCT ON (worker_id) created_at, {payload} AS payload FROM {table} WHERE run_id=$1 AND created_at > $2 ORDER BY worker_id, created_at, id) r ORDER BY created_at"
        );
        for row in sqlx::query_scalar::<_, Value>(&query)
            .bind(run_id)
            .bind(until)
            .fetch_all(store.pool())
            .await
            .map_err(crate::core::StoreError::from)?
        {
            graphs.observe(row, evaluator);
        }
    }
    let cadence = serde_json::to_value(&graphs.cadence)
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let (panels, states) = graphs.panels();
    Ok(json!({"bounds": bounds, "selection": [start, end],
        "bin_seconds": (end - start) as f64 / DISPLAY_BINS as f64 / 1000.0,
        "cadence": cadence, "panels": panels, "states": states}))
}
