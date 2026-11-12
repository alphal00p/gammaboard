use super::*;
use crate::server::panels::{
    format_bytes_human, key_value, key_value_panel, table_panel_with_payload,
};

pub(super) async fn load(
    store: &PgStore,
    run: i32,
    task: i64,
    range: &performance_panels::history::RangeQuery,
) -> Result<Value, ApiError> {
    let raw: Value=sqlx::query_scalar(r#"
        WITH children AS (SELECT id FROM runs WHERE parent_run_id=$1 AND parent_task_id=$2),
        fleet AS (
            SELECT n.name,n.uuid,n.active_role,r.metrics,r.rss_bytes,r.created_at,
                r.metrics->>'node_uuid'=n.uuid AND r.metrics->>'task_id'=t.id::text
                    AND r.metrics->>'epoch' IS NOT NULL AS matched
            FROM nodes n LEFT JOIN run_tasks t ON t.run_id=n.active_run_id AND t.state='active'
            LEFT JOIN LATERAL (
                SELECT metrics,rss_bytes,created_at FROM evaluator_performance_latest
                WHERE worker_id=n.name AND run_id=n.active_run_id AND n.active_role='evaluator'
                UNION ALL SELECT metrics,rss_bytes,created_at FROM sampler_aggregator_performance_latest
                WHERE worker_id=n.name AND run_id=n.active_run_id AND n.active_role='sampler_aggregator'
            ) r ON true
            WHERE n.lease_expires_at>now() AND n.active_run_id IN (SELECT id FROM children)
        )
        SELECT jsonb_build_object('ids',COALESCE((SELECT jsonb_agg(id) FROM children),'[]'),
            'samples',(SELECT COALESCE(sum(nr_completed_samples),0) FROM runs WHERE id IN (SELECT id FROM children)),
            'core_hours',(SELECT COALESCE(sum(task_cpu_seconds(id,cpu_seconds)),0)/3600.0 FROM run_tasks WHERE run_id IN (SELECT id FROM children)),
            'evaluators',(SELECT count(*) FROM fleet WHERE active_role='evaluator'),
            'samplers',(SELECT count(*) FROM fleet WHERE active_role='sampler_aggregator'),
            'fresh',(SELECT count(*) FROM fleet WHERE matched AND created_at BETWEEN now()-interval '10 seconds' AND now()),
            'rss',(SELECT sum(rss_bytes) FROM fleet WHERE matched AND created_at BETWEEN now()-interval '10 seconds' AND now() AND rss_bytes>=0),
            'memory_reports',(SELECT count(*) FROM fleet WHERE matched AND created_at BETWEEN now()-interval '10 seconds' AND now() AND rss_bytes>=0))
    "#).bind(run).bind(task).fetch_one(store.pool()).await.map_err(crate::core::StoreError::from)?;
    let ids: Vec<i32> = serde_json::from_value(raw["ids"].clone())
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let history = performance_panels::history::load_many(store, &ids, range, false, true).await?;
    let m = history.campaign_measurements();
    let live = raw["evaluators"].as_i64().unwrap_or(0) + raw["samplers"].as_i64().unwrap_or(0);
    Ok(response(
        format!("campaign:{run}:{task}:overview"),
        vec![
            (
                "Progress and current resources",
                key_value_panel(
                    "performance_overview",
                    vec![
                        key_value(
                            "samples",
                            "Accepted samples (all child stages)",
                            &raw["samples"],
                        ),
                        key_value(
                            "accepted_rate",
                            "Accepted samples / s (selected interval)",
                            &m["rate"],
                        ),
                        key_value(
                            "allocated_core_hours",
                            "Allocated core-hours",
                            &raw["core_hours"],
                        ),
                        key_value(
                            "live_workers",
                            "Live evaluators / samplers",
                            format!("{} / {}", raw["evaluators"], raw["samplers"]),
                        ),
                        key_value(
                            "coverage",
                            "Fresh reports / live workers",
                            format!("{} / {live}", raw["fresh"]),
                        ),
                        key_value(
                            "rss",
                            "Current GammaBoard RSS (excludes Python/GPU)",
                            raw["rss"].as_i64().map(format_bytes_human),
                        ),
                        key_value(
                            "memory_coverage",
                            "Memory reports / live workers",
                            format!("{} / {live}", raw["memory_reports"]),
                        ),
                    ],
                ),
            ),
            (
                "Activity — selected interval",
                table_panel_with_payload(
                    "busy_rates",
                    vec![
                        "Role".into(),
                        "Compute wall time (%)".into(),
                        "I/O wall time (%)".into(),
                    ],
                    vec![
                        vec![
                            json!("Evaluators"),
                            m["busy"][0].clone(),
                            m["busy"][1].clone(),
                        ],
                        vec![
                            json!("Samplers"),
                            m["busy"][2].clone(),
                            m["busy"][3].clone(),
                        ],
                    ],
                    Some(
                        json!({"description":"Activity is weighted by reporting worker-time. Throughput sums accepted counter changes over the shared interval. Gaps in retained telemetry leave incomplete coverage; missing activity is not zero."}),
                    ),
                ),
            ),
            (
                "Measurement window",
                key_value_panel(
                    "measurement_window",
                    vec![
                        key_value("children", "Child runs", ids.len()),
                        key_value(
                            "observed_seconds",
                            "Observed reporting worker-time (s)",
                            &m["worker_seconds"],
                        ),
                        key_value(
                            "window_seconds",
                            "Recorded window (s)",
                            m["selection"].as_array().and_then(|r| {
                                Some((r[1].as_i64()? - r[0].as_i64()?) as f64 / 1000.0)
                            }),
                        ),
                        key_value(
                            "coverage",
                            "Scope",
                            "All child stages; retained reporting intervals only",
                        ),
                    ],
                ),
            ),
        ],
    ))
}
