//! Counter differences in one requested wall-time window. Worker publications
//! are asynchronous; observed spans and coverage are explicit.
use crate::api::performance::PerformanceSnapshot;
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelResponse, PanelSpec, PanelState, PanelWidth,
    format_bytes_human, key_value, key_value_panel, replace_panel, sized_panel_spec,
    table_panel_with_payload, text_panel,
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

mod graphs;
pub(super) mod history;

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

    fn cost_us(&self, pointer: &str) -> Option<f64> {
        let samples = self.delta("/samples_evaluated")?;
        if samples <= 0.0 {
            return None;
        }
        self.delta(pointer).map(|v| v * 1e6 / samples)
    }
}

fn interval<'a>(
    rows: &'a [Value],
    latest: &Value,
    node: &Value,
    snapshot: &PerformanceSnapshot,
    since: DateTime<Utc>,
) -> Option<Interval<'a>> {
    if !matches_worker(latest, node, snapshot)
        || age(latest, snapshot.observed_at).is_none_or(|age| age > MAX_AGE_SECONDS)
    {
        return None;
    }
    let evaluator = node["active_role"] == "evaluator";
    let mut matching = rows.iter().filter(|r| {
        matches_worker(r, node, snapshot)
            && epoch(r, evaluator) == epoch(latest, evaluator)
            && timestamp(r).is_some_and(|t| t >= since && t <= snapshot.observed_at)
    });
    let first = matching.next()?;
    let last = matching.next_back()?;
    let seconds = (timestamp(last)? - timestamp(first)?).num_milliseconds() as f64 / 1000.0;
    if seconds <= 0.0 || age(last, snapshot.observed_at)? > MAX_AGE_SECONDS {
        return None;
    }
    let result = Interval {
        first,
        last,
        seconds,
        evaluator,
    };
    // A reset is not a zero-throughput interval.
    result.delta(if evaluator {
        "/samples_evaluated"
    } else {
        "/completed_samples_total"
    })?;
    Some(result)
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

pub(super) fn build_performance_response(
    snapshot: &PerformanceSnapshot,
    evaluator_rows: &[Value],
    sampler_rows: &[Value],
    window_seconds: i64,
    selected_node: Option<&str>,
) -> PanelResponse {
    let since = snapshot.observed_at - chrono::Duration::seconds(window_seconds);
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
    let mut interval_count = 0;
    let mut memory_count = 0;
    let mut rss = 0i64;
    let mut ages = Vec::new();
    let mut spans = Vec::new();
    let mut evaluator_busy = Vec::new();
    let mut sampler_interval = None;
    let mut sampler_latest = None;
    let mut worker_rows = Vec::new();
    let mut evaluator_details = Vec::new();
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
        let measured = fresh.and_then(|r| {
            interval(
                if evaluator {
                    evaluator_rows
                } else {
                    sampler_rows
                },
                r,
                node,
                snapshot,
                since,
            )
        });
        if let Some(value) = &measured {
            interval_count += 1;
            spans.push(value.seconds);
            if evaluator {
                evaluator_busy.push((
                    value.busy_percent("compute"),
                    value.busy_percent("io"),
                    value.busy_seconds(),
                ));
            }
        }
        let status = if matched.is_none() {
            "Missing/current identity not reported"
        } else if fresh.is_none() {
            "Stale"
        } else if measured.is_none() {
            "Waiting for two snapshots"
        } else {
            "Current"
        };
        worker_rows.push(vec![
            node["name"].clone(),
            node["active_role"].clone(),
            json!(status),
            json!(age_seconds),
            json!(measured.as_ref().map(|v| v.seconds)),
            json!(memory.map(format_bytes_human)),
        ]);
        if evaluator && selected_node.is_none_or(|name| node["name"] == name) {
            let value = measured.as_ref();
            evaluator_details.push(vec![
                node["name"].clone(),
                json!(value.and_then(|v| v.busy_percent("compute"))),
                json!(value.and_then(|v| v.busy_percent("io"))),
                json!(value.and_then(|v| v.delta("/samples_evaluated"))),
                json!(value.and_then(|v| v.cost_us("/cumulative/evaluate_seconds"))),
                json!(value.and_then(|v| v.cost_us("/cumulative/materialize_seconds"))),
                json!(value.and_then(|v| v.cost_us("/cumulative/fetch_wait_seconds"))),
                json!(value.and_then(|v| v.cost_us("/cumulative/submit_seconds"))),
                json!(value.and_then(|v| v.cost_us("/cumulative/submit_wait_seconds"))),
            ]);
        }
        if !evaluator {
            sampler_interval = measured;
            sampler_latest = fresh;
        }
    }
    let evaluator_count = nodes
        .iter()
        .filter(|n| n["active_role"] == "evaluator")
        .count();
    let sampler_count = nodes.len() - evaluator_count;
    if sampler_count != 1 {
        sampler_interval = None;
        sampler_latest = None;
    }
    let sampler_windows = sampler_interval
        .as_ref()
        .map(|interval| graphs::sampler_rows_in_interval(sampler_rows, interval))
        .unwrap_or_default();
    let rate = sampler_interval
        .as_ref()
        .and_then(|v| v.delta("/completed_samples_total").map(|n| n / v.seconds));
    let eval_busy = |io: bool| -> Option<f64> {
        if evaluator_count == 0 || evaluator_busy.len() != evaluator_count {
            return None;
        }
        let (mut total, mut elapsed) = (0.0, 0.0);
        for &(compute, input_output, seconds) in &evaluator_busy {
            let seconds = seconds?;
            total += if io { input_output? } else { compute? } * seconds;
            elapsed += seconds;
        }
        Some(total / elapsed)
    };
    let mut panels = Vec::new();
    let mut states = Vec::new();
    add_panel(
        &mut panels,
        &mut states,
        "performance_overview",
        "Usage overview",
        PanelKind::KeyValue,
        key_value_panel(
            "performance_overview",
            vec![
                key_value(
                    "accepted_samples",
                    "Accepted Samples",
                    snapshot.completed_samples,
                ),
                key_value("accepted_rate", "Accepted Samples / s", rate),
                key_value(
                    "live_workers",
                    "Live Workers (evaluators / samplers)",
                    format!("{evaluator_count} / {sampler_count}"),
                ),
                key_value(
                    "coverage",
                    "Fresh Reports / Live Workers",
                    format!("{fresh_count} / {}", nodes.len()),
                ),
                key_value(
                    "interval_coverage",
                    "Workers With Measured Intervals",
                    format!("{interval_count} / {}", nodes.len()),
                ),
                key_value(
                    "allocated_core_hours",
                    "Allocated Core-hours (run total)",
                    snapshot.allocated_core_seconds / 3600.0,
                ),
                key_value(
                    "process_rss",
                    "Reported GammaBoard Process RSS",
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
        "busy_rates",
        "Activity",
        PanelKind::Table,
        table_panel_with_payload(
            "busy_rates",
            ["Role", "Compute busy (%)", "I/O active (%)"]
                .map(String::from)
                .to_vec(),
            vec![
                vec![
                    json!("Evaluators"),
                    json!(eval_busy(false)),
                    json!(eval_busy(true)),
                ],
                vec![
                    json!("Sampler"),
                    json!(
                        sampler_interval
                            .as_ref()
                            .and_then(|v| v.busy_percent("compute"))
                    ),
                    json!(sampler_interval.as_ref().and_then(|v| v.busy_percent("io"))),
                ],
            ],
            None,
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
                key_value("since", "From", since),
                key_value("until", "To", snapshot.observed_at),
                key_value("window_seconds", "Requested Window (s)", window_seconds),
                key_value(
                    "observed_seconds",
                    "Shortest Observed Worker Interval (s)",
                    spans.into_iter().reduce(f64::min),
                ),
                key_value(
                    "scope",
                    "Scope",
                    "Activity uses measured worker-time within this window, with full live-worker coverage. Compute and I/O can overlap; I/O stops at completion. These are operation wall times. RSS excludes subprocesses, GPUs and services.",
                ),
            ],
        ),
    );
    add_panel(
        &mut panels,
        &mut states,
        "worker_coverage",
        "Worker reporting coverage",
        PanelKind::Table,
        table_panel_with_payload(
            "worker_coverage",
            [
                "Worker",
                "Role",
                "Telemetry",
                "Age (s)",
                "Observed interval (s)",
                "Process RSS",
            ]
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
        "Evaluator operations — separate wall durations",
        PanelKind::Table,
        table_panel_with_payload(
            "evaluator_diagnostics",
            [
                "Worker",
                "Compute busy (%)",
                "I/O active (%)",
                "Submitted samples",
                "Evaluate (µs/sample)",
                "Materialize (µs/sample)",
                "Exposed fetch wait (µs/sample)",
                "Async submit latency (µs/sample)",
                "Exposed submit wait (µs/sample)",
            ]
            .map(String::from)
            .to_vec(),
            evaluator_details,
            None,
        ),
    );

    if let Some(latest) = sampler_latest {
        sampler_queue_panels(&mut panels, &mut states, latest, snapshot.observed_at);
    }
    if sampler_interval.is_some() {
        add_sampler_timings(&mut panels, &mut states, &sampler_windows);
    } else {
        add_panel(
            &mut panels,
            &mut states,
            "sampler_diagnostics_status",
            "Sampler diagnostics",
            PanelKind::Text,
            text_panel(
                "sampler_diagnostics_status",
                "No current sampler measurement interval is available.",
            ),
        );
    }
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

fn add_sampler_timings(panels: &mut Vec<PanelSpec>, states: &mut Vec<PanelState>, rows: &[&Value]) {
    let mut timing_rows = Vec::new();
    for (path, label, unit) in [
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
    ] {
        let mut count = 0u64;
        let mut total = 0.0;
        for row in rows {
            let metric = data(row, false).pointer(path).unwrap_or(&Value::Null);
            if let (Some(n), Some(sum)) = (metric["count"].as_u64(), finite(&metric["total"])) {
                count += n;
                total += sum;
            }
        }
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
        "Sampler and I/O observations in the measured interval",
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
