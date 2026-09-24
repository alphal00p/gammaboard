//! Machine-readable performance data shared by the CLI and HTTP API.
//! Rates use accepted progress counters. Dashboard rolling metrics remain diagnostic.
use crate::PgStore;
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceSnapshot {
    pub schema_version: u32,
    pub observed_at: DateTime<Utc>,
    pub run_id: i32,
    pub run_name: String,
    pub task_id: Option<String>,
    pub completed_samples: i64,
    pub allocated_core_seconds: f64,
    pub failed_tasks: i64,
    pub unfinished_tasks: i64,
    pub queue: Value,
    pub nodes: Vec<Value>,
    pub evaluators: Vec<Value>,
    pub samplers: Vec<Value>,
}

/// One SQL statement gives a consistent database view, but worker publications
/// remain asynchronous. Their timestamps and epochs are returned unmodified.
pub async fn snapshot(store: &PgStore, run_id: i32) -> Result<PerformanceSnapshot> {
    let raw: Option<Value> = sqlx::query_scalar(r#"
        SELECT jsonb_build_object(
            'schema_version', 1, 'observed_at', statement_timestamp(),
            'run_id', r.id, 'run_name', r.name,
            'task_id', (SELECT id::text FROM run_tasks WHERE run_id=r.id AND state='active' LIMIT 1),
            'completed_samples', r.nr_completed_samples,
            'allocated_core_seconds', (SELECT COALESCE(SUM(cpu_seconds),0) FROM run_tasks WHERE run_id=r.id),
            'failed_tasks', (SELECT count(*) FROM run_tasks WHERE run_id=r.id AND state='failed'),
            'unfinished_tasks', (SELECT count(*) FROM run_tasks WHERE run_id=r.id AND state NOT IN ('completed','failed')),
            'queue', COALESCE((SELECT to_jsonb(q)-'run_id' FROM run_batch_queue_counters q WHERE q.run_id=r.id),'{}'),
            'nodes', COALESCE((SELECT jsonb_agg(jsonb_build_object(
                'name', n.name, 'uuid', n.uuid, 'live', n.lease_expires_at>statement_timestamp(),
                'pool_run_id', n.pool_run_id, 'pool_role', n.pool_role,
                'desired_run_id', n.desired_run_id, 'desired_role', n.desired_role,
                'active_run_id', n.active_run_id, 'active_role', n.active_role,
                'capabilities', n.capabilities, 'last_seen', n.last_seen
            ) ORDER BY n.name) FROM nodes n WHERE n.pool_run_id=r.id OR n.active_run_id=r.id OR n.desired_run_id=r.id),'[]'),
            'evaluators', COALESCE((SELECT jsonb_agg(to_jsonb(e) ORDER BY e.worker_id)
                FROM evaluator_performance_latest e WHERE e.run_id=r.id),'[]'),
            'samplers', COALESCE((SELECT jsonb_agg(to_jsonb(s) ORDER BY s.worker_id)
                FROM sampler_aggregator_performance_latest s WHERE s.run_id=r.id),'[]')
        ) FROM runs r WHERE r.id=$1
    "#).bind(run_id).fetch_optional(store.pool()).await?;
    serde_json::from_value(raw.context("run does not exist")?)
        .context("invalid performance snapshot")
}

impl PerformanceSnapshot {
    pub fn active_evaluators(&self) -> usize {
        self.nodes
            .iter()
            .filter(|n| {
                n["live"] == true
                    && n["active_run_id"] == self.run_id
                    && n["active_role"] == "evaluator"
            })
            .count()
    }

    pub fn ready(&self, evaluators: usize, max_age: Duration) -> bool {
        self.task_id.is_some()
            && self.failed_tasks == 0
            && self.active_evaluators() == evaluators
            && self
                .nodes
                .iter()
                .filter(|n| {
                    n["live"] == true
                        && n["active_run_id"] == self.run_id
                        && n["active_role"] == "sampler_aggregator"
                })
                .count()
                == 1
            && self.coverage_issues(max_age).is_empty()
    }

    pub fn idle(&self) -> bool {
        !self.nodes.iter().any(|n| {
            n["live"] == true
                && (n["active_run_id"] == self.run_id || n["desired_run_id"] == self.run_id)
        })
    }

    fn fleet(&self) -> Vec<Value> {
        self.nodes
            .iter()
            .map(|n| {
                json!([
                    n["name"],
                    n["uuid"],
                    n["live"],
                    n["active_run_id"],
                    n["active_role"],
                    n["desired_run_id"],
                    n["desired_role"]
                ])
            })
            .collect()
    }

    fn epochs(&self) -> Vec<Value> {
        self.nodes
            .iter()
            .filter(|n| n["live"] == true && n["active_role"] == "evaluator")
            .map(|n| {
                let row = self.evaluators.iter().find(|e| e["worker_id"] == n["name"]);
                json!([n["uuid"], row.map(|e| &e["metrics"]["epoch"])])
            })
            .collect()
    }

    pub fn coverage_issues(&self, max_age: Duration) -> Vec<String> {
        let mut issues = Vec::new();
        for n in &self.nodes {
            let name = n["name"].as_str().unwrap_or("unknown");
            if n["live"] != true
                || n["active_run_id"] != self.run_id
                || n["desired_run_id"] != n["active_run_id"]
                || n["desired_role"] != n["active_role"]
            {
                issues.push(format!("unsettled assignment: {name}"));
                continue;
            }
            let is_evaluator = n["active_role"] == "evaluator";
            let rows = if is_evaluator {
                &self.evaluators
            } else {
                &self.samplers
            };
            match rows.iter().find(|e| e["worker_id"] == n["name"]) {
                None => issues.push(format!("missing telemetry: {name}")),
                Some(e) => {
                    if !is_evaluator
                        && (e["runtime_metrics"]["runner_epoch"].as_str().is_none()
                            || e["runtime_metrics"]["task_id"].as_str() != self.task_id.as_deref())
                    {
                        issues.push(format!("unknown or mismatched sampler epoch: {name}"));
                    }
                    let timestamp = e["created_at"]
                        .as_str()
                        .and_then(|v| DateTime::parse_from_rfc3339(v).ok());
                    if timestamp.is_none_or(|t| {
                        (self.observed_at - t.with_timezone(&Utc)).num_milliseconds() as f64
                            > max_age.as_secs_f64() * 1000.0
                    }) {
                        issues.push(format!("stale telemetry: {name}"));
                    }
                    if is_evaluator
                        && (e["metrics"]["epoch"].as_str().is_none()
                            || e["metrics"]["node_uuid"] != n["uuid"]
                            || e["metrics"]["task_id"].as_str() != self.task_id.as_deref())
                    {
                        issues.push(format!("unknown or mismatched evaluator epoch: {name}"));
                    }
                }
            }
        }
        if self
            .nodes
            .iter()
            .filter(|n| {
                n["live"] == true
                    && n["active_run_id"] == self.run_id
                    && n["active_role"] == "sampler_aggregator"
            })
            .count()
            != 1
        {
            issues.push("expected one active sampler".into());
        }
        if self.active_evaluators() == 0 {
            issues.push("no active evaluators".into());
        }
        issues
    }
}

#[derive(Debug, Serialize)]
pub struct PerformanceInterval {
    pub schema_version: u32,
    pub elapsed_seconds: f64,
    pub completed_samples: i64,
    pub samples_per_second: Option<f64>,
    pub allocated_core_seconds: f64,
    pub valid: bool,
    pub issues: Vec<String>,
    pub observed_assignment_changes: usize,
    pub observed_evaluator_epoch_changes: usize,
    pub evaluator_deltas: Vec<Value>,
    pub snapshots: Vec<PerformanceSnapshot>,
}

pub fn summarize(
    snapshots: Vec<PerformanceSnapshot>,
    elapsed_seconds: f64,
    max_age: Duration,
) -> PerformanceInterval {
    let mut issues = Vec::new();
    let mut assignment_changes = 0;
    let mut epoch_changes = 0;
    for s in &snapshots {
        issues.extend(s.coverage_issues(max_age));
        if s.task_id.is_none() {
            issues.push("no active task during measurement".into());
        }
        if s.failed_tasks > 0 {
            issues.push("run contains a failed task".into());
        }
    }
    for pair in snapshots.windows(2) {
        if pair[0].run_id != pair[1].run_id || pair[0].task_id != pair[1].task_id {
            issues.push("run or task changed".into());
        }
        let sampler_epochs = |s: &PerformanceSnapshot| {
            s.samplers
                .iter()
                .map(|r| json!([r["worker_id"], r["runtime_metrics"]["runner_epoch"]]))
                .collect::<Vec<_>>()
        };
        if sampler_epochs(&pair[0]) != sampler_epochs(&pair[1]) {
            issues.push("sampler incarnations changed".into());
        }
        if pair[0].completed_samples > pair[1].completed_samples {
            issues.push("accepted sample counter decreased".into());
        }
        assignment_changes += usize::from(pair[0].fleet() != pair[1].fleet());
        epoch_changes += usize::from(pair[0].epochs() != pair[1].epochs());
    }
    if assignment_changes > 0 {
        issues.push("worker assignments changed".into());
    }
    if epoch_changes > 0 {
        issues.push("evaluator incarnations changed".into());
    }
    if snapshots.len() < 2 || !elapsed_seconds.is_finite() || elapsed_seconds <= 0.0 {
        issues.push("insufficient observation interval".into());
    }
    let completed_samples = snapshots
        .last()
        .zip(snapshots.first())
        .map_or(0, |(b, a)| b.completed_samples - a.completed_samples);
    let allocated_core_seconds = snapshots
        .last()
        .zip(snapshots.first())
        .map_or(0.0, |(b, a)| {
            b.allocated_core_seconds - a.allocated_core_seconds
        });
    let mut evaluator_deltas = Vec::new();
    if let Some((first, last)) = snapshots.first().zip(snapshots.last()) {
        for end in &last.evaluators {
            let Some(start) = first.evaluators.iter().find(|s| {
                s["worker_id"] == end["worker_id"]
                    && s["metrics"]["epoch"].is_string()
                    && s["metrics"]["epoch"] == end["metrics"]["epoch"]
            }) else {
                continue;
            };
            let mut delta = json!({"worker_id":end["worker_id"],"epoch":end["metrics"]["epoch"],
                "since":start["created_at"],"until":end["created_at"]});
            for key in ["samples_evaluated", "batches_completed"] {
                if let Some((a, b)) = start["metrics"][key]
                    .as_i64()
                    .zip(end["metrics"][key].as_i64())
                {
                    if b < a {
                        issues.push("evaluator counter decreased".into());
                    }
                    delta[key] = json!(b - a);
                }
            }
            for key in [
                "evaluate_seconds",
                "materialize_seconds",
                "fetch_wait_seconds",
                "submit_seconds",
                "submit_wait_seconds",
            ] {
                if let Some((a, b)) = start["metrics"]["cumulative"][key]
                    .as_f64()
                    .zip(end["metrics"]["cumulative"][key].as_f64())
                {
                    if b < a {
                        issues.push("evaluator timing counter decreased".into());
                    }
                    delta[key] = json!(b - a);
                }
            }
            evaluator_deltas.push(delta);
        }
    }
    issues.sort();
    issues.dedup();
    PerformanceInterval {
        schema_version: 1,
        elapsed_seconds,
        completed_samples,
        samples_per_second: issues
            .is_empty()
            .then(|| completed_samples as f64 / elapsed_seconds),
        allocated_core_seconds,
        valid: issues.is_empty(),
        issues,
        observed_assignment_changes: assignment_changes,
        observed_evaluator_epoch_changes: epoch_changes,
        evaluator_deltas,
        snapshots,
    }
}

pub async fn measure(
    store: &PgStore,
    run_id: i32,
    duration: Duration,
    interval: Duration,
    max_age: Duration,
) -> Result<PerformanceInterval> {
    ensure!(
        !duration.is_zero() && !interval.is_zero(),
        "durations must be positive"
    );
    ensure!(
        duration.as_secs_f64() / interval.as_secs_f64() <= 10000.0,
        "interval would exceed 10000 snapshots; increase --interval"
    );
    let start_query = Instant::now();
    let first = snapshot(store, run_id).await?;
    let start = start_query + start_query.elapsed() / 2;
    let mut snapshots = vec![first];
    let mut end = start;
    while end.duration_since(start) < duration {
        tokio::time::sleep(interval.min(duration.saturating_sub(end.duration_since(start)))).await;
        let before = Instant::now();
        snapshots.push(snapshot(store, run_id).await?);
        end = before + before.elapsed() / 2;
    }
    Ok(summarize(
        snapshots,
        end.duration_since(start).as_secs_f64(),
        max_age,
    ))
}

pub async fn history(
    store: &PgStore,
    run_id: i32,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    limit: i64,
) -> Result<Value> {
    ensure!(
        since < until && (1..=10000).contains(&limit),
        "history needs since < until and limit in 1..=10000"
    );
    let mut rows: Vec<Value> = sqlx::query_scalar(r#"
        SELECT row FROM (
            SELECT created_at, id, 'evaluator' AS role, to_jsonb(e) || '{"role":"evaluator"}'::jsonb AS row
            FROM evaluator_performance_history e WHERE run_id=$1 AND created_at >= $2 AND created_at < $3
            UNION ALL
            SELECT created_at, id, 'sampler' AS role, to_jsonb(s) || '{"role":"sampler"}'::jsonb AS row
            FROM sampler_aggregator_performance_history s WHERE run_id=$1 AND created_at >= $2 AND created_at < $3
        ) entries ORDER BY created_at, role, id LIMIT $4
    "#).bind(run_id).bind(since).bind(until).bind(limit+1).fetch_all(store.pool()).await?;
    let truncated = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    Ok(
        json!({"schema_version":1,"run_id":run_id,"since":since,"until":until,"truncated":truncated,"rows":rows}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> PerformanceSnapshot {
        let now = Utc::now();
        let node = |name: &str, role: &str| {
            json!({"name":name,"uuid":name,"live":true,
            "active_run_id":1,"desired_run_id":1,"active_role":role,"desired_role":role})
        };
        PerformanceSnapshot {
            schema_version: 1,
            observed_at: now,
            run_id: 1,
            run_name: "test".into(),
            task_id: Some("7".into()),
            completed_samples: 100,
            allocated_core_seconds: 1.,
            failed_tasks: 0,
            unfinished_tasks: 1,
            queue: json!({}),
            nodes: vec![node("e", "evaluator"), node("s", "sampler_aggregator")],
            evaluators: vec![
                json!({"worker_id":"e","created_at":now,"metrics":{"epoch":"one","node_uuid":"e","task_id":"7","samples_evaluated":100,"batches_completed":1,
                "cumulative":{"evaluate_seconds":0.1}}}),
            ],
            samplers: vec![
                json!({"worker_id":"s","created_at":now,"runtime_metrics":{"runner_epoch":"sampler-one","task_id":"7"}}),
            ],
        }
    }
    #[test]
    fn measures_counters_and_rejects_restart_stale_and_task_change() {
        let a = sample();
        let mut b = a.clone();
        b.completed_samples = 300;
        let good = summarize(vec![a.clone(), b.clone()], 2., Duration::from_secs(10));
        assert!(good.valid);
        assert_eq!(good.samples_per_second, Some(100.));
        b.evaluators[0]["metrics"]["epoch"] = json!("two");
        let restart = summarize(vec![a.clone(), b], 2., Duration::from_secs(10));
        assert!(!restart.valid);
        assert_eq!(restart.samples_per_second, None);
        assert_eq!(restart.observed_evaluator_epoch_changes, 1);
        let mut stale = a.clone();
        stale.observed_at += chrono::Duration::seconds(30);
        assert!(!stale.ready(1, Duration::from_secs(10)));
        let mut changed = a.clone();
        changed.task_id = Some("8".into());
        assert!(!summarize(vec![a.clone(), changed], 2., Duration::from_secs(10)).valid);
        let mut reset = a.clone();
        reset.completed_samples = 0;
        assert!(!summarize(vec![a.clone(), reset], 2., Duration::from_secs(10)).valid);
        let mut sampler_restart = a.clone();
        sampler_restart.samplers[0]["runtime_metrics"]["runner_epoch"] = json!("two");
        assert!(!summarize(vec![a, sampler_restart], 2., Duration::from_secs(10)).valid);
    }
}
