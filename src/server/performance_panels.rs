//! Counter differences in one requested wall-time window. Worker publications
//! are asynchronous; observed spans and coverage are explicit.
use crate::api::performance::PerformanceSnapshot;
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelResponse, PanelSpec, PanelState, PanelWidth,
    format_bytes_human, key_value, key_value_panel, replace_panel, sized_panel_spec,
    table_panel_with_payload,
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

mod graphs;
pub(super) mod history;
mod measurements;

const MAX_AGE_SECONDS: f64 = 10.0;

fn timestamp(row: &Value) -> Option<DateTime<Utc>> {
    row["created_at"].as_str()?.parse().ok()
}

fn age(row: &Value, now: DateTime<Utc>) -> Option<f64> {
    let age = (now - timestamp(row)?).num_milliseconds() as f64 / 1000.0;
    (age >= 0.0).then_some(age)
}

fn finite(value: &Value) -> Option<f64> {
    value.as_f64().filter(|v| v.is_finite() && *v >= 0.0)
}

fn data(row: &Value, evaluator: bool) -> &Value {
    &row[if evaluator {
        "metrics"
    } else {
        "runtime_metrics"
    }]
}

fn epoch(row: &Value, evaluator: bool) -> Option<&str> {
    data(row, evaluator)[if evaluator { "epoch" } else { "runner_epoch" }].as_str()
}

fn matches_worker(row: &Value, node: &Value, snapshot: &PerformanceSnapshot) -> bool {
    let evaluator = node["active_role"] == "evaluator";
    let metrics = data(row, evaluator);
    row["worker_id"] == node["name"]
        && epoch(row, evaluator).is_some()
        && metrics["node_uuid"].is_string()
        && metrics["node_uuid"] == node["uuid"]
        && snapshot.task_id.is_some()
        && metrics["task_id"].as_str() == snapshot.task_id.as_deref()
}

struct Interval<'a> {
    first: &'a Value,
    last: &'a Value,
    seconds: f64,
    evaluator: bool,
}

impl Interval<'_> {
    fn delta(&self, pointer: &str) -> Option<f64> {
        let first = data(self.first, self.evaluator).pointer(pointer)?;
        let last = data(self.last, self.evaluator).pointer(pointer)?;
        if let (Some(a), Some(b)) = (first.as_i64(), last.as_i64()) {
            return b.checked_sub(a).filter(|v| *v >= 0).map(|v| v as f64);
        }
        let a = finite(first)?;
        let b = finite(last)?;
        (b >= a).then_some(b - a)
    }

    fn busy_seconds(&self) -> Option<f64> {
        self.delta("/busy/elapsed_seconds")
            .filter(|seconds| *seconds > 0.0)
    }

    fn busy_percent(&self, lane: &str) -> Option<f64> {
        let seconds = self.busy_seconds()?;
        let busy = self.delta(&format!("/busy/{lane}_seconds"))?;
        // Both counters use the same monotonic clock; tolerate only floating-point error.
        (busy <= seconds + 1e-9).then_some(100.0 * (busy / seconds).min(1.0))
    }
}

fn add_panel(
    panels: &mut Vec<PanelSpec>,
    states: &mut Vec<PanelState>,
    id: &str,
    label: &str,
    kind: PanelKind,
    state: PanelState,
) {
    panels.push(sized_panel_spec(
        id,
        label,
        kind,
        PanelHistoryMode::Replace,
        PanelWidth::Full,
    ));
    states.push(state);
}

fn build_performance_response(
    snapshot: &PerformanceSnapshot,
    measurement: &measurements::Measurements,
    range: Option<[i64; 2]>,
    selected_node: Option<&str>,
) -> PanelResponse {
    let nodes = snapshot
        .nodes
        .iter()
        .filter(|n| {
            n["live"] == true
                && n["active_run_id"] == snapshot.run_id
                && matches!(
                    n["active_role"].as_str(),
                    Some("evaluator" | "sampler_aggregator")
                )
        })
        .collect::<Vec<_>>();
    let mut fresh_count = 0;
    let mut memory_count = 0;
    let mut rss = 0i64;
    let mut ages = Vec::new();
    let mut sampler_latest = None;
    let mut worker_rows = Vec::new();
    for node in &nodes {
        let evaluator = node["active_role"] == "evaluator";
        let latest = if evaluator {
            &snapshot.evaluators
        } else {
            &snapshot.samplers
        }
        .iter()
        .find(|r| r["worker_id"] == node["name"]);
        let matched = latest.filter(|r| matches_worker(r, node, snapshot));
        let age_seconds = matched.and_then(|r| age(r, snapshot.observed_at));
        ages.extend(age_seconds);
        let fresh = matched.filter(|_| age_seconds.is_some_and(|v| v <= MAX_AGE_SECONDS));
        fresh_count += usize::from(fresh.is_some());
        let memory = fresh
            .and_then(|r| r["rss_bytes"].as_i64())
            .filter(|v| *v >= 0);
        if let Some(bytes) = memory {
            rss = rss.saturating_add(bytes);
            memory_count += 1;
        }
        let status = if matched.is_none() {
            "Missing/current identity not reported"
        } else if fresh.is_none() {
            "Stale"
        } else {
            "Current"
        };
        worker_rows.push(vec![
            node["name"].clone(),
            node["active_role"].clone(),
            json!(status),
            json!(age_seconds),
            json!(memory.map(format_bytes_human)),
        ]);
        if !evaluator {
            sampler_latest = fresh;
        }
    }
    let evaluator_count = nodes
        .iter()
        .filter(|n| n["active_role"] == "evaluator")
        .count();
    let sampler_count = nodes.len() - evaluator_count;
    if sampler_count != 1 {
        sampler_latest = None;
    }
    let mut panels = Vec::new();
    let mut states = Vec::new();
    add_panel(
        &mut panels,
        &mut states,
        "busy_rates",
        "Activity — selected interval",
        PanelKind::Table,
        table_panel_with_payload(
            "busy_rates",
            ["Role", "Compute busy (%)", "I/O active (%)"]
                .map(String::from)
                .to_vec(),
            vec![
                vec![
                    json!("Evaluators"),
                    json!(measurement.busy[0].value()),
                    json!(measurement.busy[1].value()),
                ],
                vec![
                    json!("Sampler"),
                    json!(measurement.busy[2].value()),
                    json!(measurement.busy[3].value()),
                ],
            ],
            None,
        ),
    );
    add_panel(
        &mut panels,
        &mut states,
        "performance_overview",
        "Progress and current resources",
        PanelKind::KeyValue,
        key_value_panel(
            "performance_overview",
            vec![
                key_value(
                    "accepted_samples",
                    "Accepted Samples (run total)",
                    snapshot.completed_samples,
                ),
                key_value(
                    "accepted_rate",
                    "Accepted Samples / s (selected interval)",
                    measurement.rate.value(),
                ),
                key_value(
                    "live_workers",
                    "Live Workers Now (evaluators / samplers)",
                    format!("{evaluator_count} / {sampler_count}"),
                ),
                key_value(
                    "coverage",
                    "Fresh Reports / Live Workers",
                    format!("{fresh_count} / {}", nodes.len()),
                ),
                key_value(
                    "allocated_core_hours",
                    "Allocated Core-hours (run total)",
                    snapshot.allocated_core_seconds / 3600.0,
                ),
                key_value(
                    "process_rss",
                    "Current GammaBoard Process RSS",
                    (memory_count > 0).then(|| format_bytes_human(rss)),
                ),
                key_value(
                    "memory_coverage",
                    "Memory Reports / Live Workers",
                    format!("{memory_count} / {}", nodes.len()),
                ),
                key_value(
                    "age",
                    "Oldest Matching Report Age (s)",
                    ages.into_iter().reduce(f64::max),
                ),
            ],
        ),
    );
    add_panel(
        &mut panels,
        &mut states,
        "measurement_window",
        "Measurement window",
        PanelKind::KeyValue,
        key_value_panel(
            "measurement_window",
            vec![
                key_value(
                    "since",
                    "From",
                    range.and_then(|r| DateTime::from_timestamp_millis(r[0])),
                ),
                key_value(
                    "until",
                    "To",
                    range.and_then(|r| DateTime::from_timestamp_millis(r[1])),
                ),
                key_value(
                    "window_seconds",
                    "Recorded Window (s)",
                    range.map(|r| (r[1] - r[0]) as f64 / 1000.0),
                ),
                key_value(
                    "observed_seconds",
                    "Observed Reporting Worker-time (s)",
                    measurement.workers.values().map(|w| w.seconds).sum::<f64>(),
                ),
                key_value(
                    "measured_workers",
                    "Reporting Workers in Interval (evaluators / samplers)",
                    format!(
                        "{} / {}",
                        measurement.workers.keys().filter(|(e, _)| *e).count(),
                        measurement.workers.keys().filter(|(e, _)| !*e).count()
                    ),
                ),
                key_value(
                    "scope",
                    "Scope",
                    "Activity and rates cover recorded reporting intervals; missing time is not zero. Boundary intervals are apportioned uniformly, as in the graphs. Compute and I/O can overlap; I/O stops at completion. These are operation wall times. Worker status, memory and queue state are current; accepted progress and core-hours are run totals. RSS excludes subprocesses, GPUs and services.",
                ),
            ],
        ),
    );
    add_panel(
        &mut panels,
        &mut states,
        "worker_coverage",
        "Current worker reporting coverage",
        PanelKind::Table,
        table_panel_with_payload(
            "worker_coverage",
            ["Worker", "Role", "Telemetry", "Age (s)", "Process RSS"]
                .map(String::from)
                .to_vec(),
            worker_rows,
            None,
        ),
    );
    add_panel(
        &mut panels,
        &mut states,
        "evaluator_diagnostics",
        "Evaluator operations — selected interval",
        PanelKind::Table,
        table_panel_with_payload(
            "evaluator_diagnostics",
            [
                "Worker",
                "Compute busy (%)",
                "I/O active (%)",
                "Submitted samples (boundary estimate)",
                "Evaluate (µs/sample)",
                "Materialize (µs/sample)",
                "Exposed fetch wait (µs/sample)",
                "Async submit latency (µs/sample)",
                "Exposed submit wait (µs/sample)",
            ]
            .map(String::from)
            .to_vec(),
            measurement
                .workers
                .iter()
                .filter(|((evaluator, name), _)| {
                    *evaluator && selected_node.is_none_or(|selected| selected == name)
                })
                .map(|((_, name), worker)| worker.row(name))
                .collect(),
            None,
        ),
    );

    if let Some(latest) = sampler_latest {
        sampler_queue_panels(&mut panels, &mut states, latest, snapshot.observed_at);
    }
    add_sampler_timings(&mut panels, &mut states, &measurement.timings);
    PanelResponse::new(
        format!("run:{}:performance", snapshot.run_id),
        None,
        panels,
        states.into_iter().map(replace_panel).collect(),
        Some(5000),
    )
}

fn sampler_queue_panels(
    panels: &mut Vec<PanelSpec>,
    states: &mut Vec<PanelState>,
    latest: &Value,
    now: DateTime<Utc>,
) {
    let runtime = data(latest, false);
    let q = &runtime["queue"];
    let claim_age = q["blocker"]["claimed_at"]
        .as_str()
        .and_then(|v| v.parse::<DateTime<Utc>>().ok())
        .map(|v| (now - v).num_milliseconds().max(0) as f64 / 1000.0);
    add_panel(
        panels,
        states,
        "queue_diagnostics",
        "Current queue and batch state",
        PanelKind::KeyValue,
        key_value_panel(
            "queue_diagnostics",
            vec![
                key_value(
                    "batch_size",
                    "Current Batch Size",
                    &runtime["batch_size_current"],
                ),
                key_value("pending", "Pending Batches", &q["db_pending_batches"]),
                key_value("claimed", "Claimed Batches", &q["db_claimed_batches"]),
                key_value(
                    "completed",
                    "Completed Batches Awaiting Ingestion",
                    &q["db_completed_batches"],
                ),
                key_value(
                    "local_pending",
                    "Local Pending Batches",
                    &q["local_pending_batches"],
                ),
                key_value(
                    "insert_tasks",
                    "Occupied Insert Slots",
                    &q["local_inflight_insert_tasks"],
                ),
                key_value(
                    "insert_batches",
                    "Batches in Insert Slots",
                    &q["local_inflight_insert_batches"],
                ),
                key_value(
                    "ready",
                    "Buffered Completed Batches",
                    &q["local_ready_processed_batches"],
                ),
                key_value(
                    "blocker",
                    "First Unfinished Batch",
                    &q["blocker"]["batch_id"],
                ),
                key_value("worker", "Claimed By", &q["blocker"]["node_name"]),
                key_value("claim_age", "Claim Age (s)", claim_age),
            ],
        ),
    );
}

const SAMPLER_TIMINGS: &[(&str, &str, &str)] = &[
    (
        "/sampler/eval_ms_per_batch",
        "Evaluate + materialize",
        "batch",
    ),
    (
        "/sampler/eval_ms_per_sample",
        "Evaluate + materialize",
        "sample",
    ),
    (
        "/sampler/produce_ms_per_sample",
        "Sample generation",
        "sample",
    ),
    (
        "/sampler/training_ingest_ms_per_sample",
        "Training ingestion",
        "sample",
    ),
    (
        "/sampler/completed_training_ingest_ms",
        "Training ingestion",
        "pass",
    ),
    (
        "/sampler/completed_merge_ingest_ms",
        "Accumulator merge",
        "pass",
    ),
    (
        "/sampler/persist_accumulator_ms",
        "Accumulator persistence",
        "operation",
    ),
    (
        "/sampler/completed_delete_ms",
        "Completed batch cleanup",
        "operation",
    ),
    (
        "/sampler/reclaim_ms",
        "Abandoned batch reclaim",
        "operation",
    ),
    (
        "/queue/rolling/fetch_completed_ms",
        "Result fetch",
        "operation",
    ),
    (
        "/queue/rolling/insert_bundle_ms",
        "Insert bundle",
        "operation",
    ),
    (
        "/queue/rolling/insert_bundle_ms_per_batch",
        "Insert bundle",
        "batch",
    ),
    (
        "/queue/rolling/insert_bundle_serialize_ms",
        "Insert serialization",
        "operation",
    ),
    (
        "/queue/rolling/insert_bundle_db_batches_ms",
        "Batch SQL",
        "operation",
    ),
    (
        "/queue/rolling/insert_bundle_db_inputs_ms",
        "Input SQL",
        "operation",
    ),
    (
        "/queue/rolling/insert_bundle_commit_ms",
        "Insert commit",
        "operation",
    ),
];

fn add_sampler_timings(
    panels: &mut Vec<PanelSpec>,
    states: &mut Vec<PanelState>,
    timings: &std::collections::BTreeMap<&str, (u64, f64)>,
) {
    let mut timing_rows = Vec::new();
    for &(path, label, unit) in SAMPLER_TIMINGS {
        let (count, total) = timings.get(path).copied().unwrap_or_default();
        if count > 0 {
            timing_rows.push(vec![
                json!(label),
                json!(unit),
                json!(count),
                json!(total),
                json!(total / count as f64),
            ]);
        }
    }
    add_panel(
        panels,
        states,
        "sampler_operation_timings",
        "Sampler observations — complete reporting intervals only",
        PanelKind::Table,
        table_panel_with_payload(
            "sampler_operation_timings",
            [
                "Operation",
                "Unit",
                "Count",
                "Total duration (ms)",
                "Mean (ms/unit)",
            ]
            .map(String::from)
            .to_vec(),
            timing_rows,
            None,
        ),
    );
}

#[cfg(test)]
mod tests;
