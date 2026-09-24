use super::*;

fn fixture() -> (PerformanceSnapshot, Vec<Value>, Vec<Value>) {
    let now = DateTime::from_timestamp(100, 0).unwrap();
    let mut snapshot: PerformanceSnapshot = serde_json::from_value(json!({
        "schema_version":1,"observed_at":now,"run_id":1,"run_name":"test","task_id":"2",
        "completed_samples":480,"allocated_core_seconds":7200.0,"failed_tasks":0,
        "unfinished_tasks":1,"queue":{},"nodes":[],"evaluators":[],"samplers":[]
    }))
    .unwrap();
    let mut evaluators = Vec::new();
    let mut samplers = Vec::new();
    for (name, role) in [("e", "evaluator"), ("s", "sampler_aggregator")] {
        snapshot.nodes.push(
            json!({"name":name,"uuid":name,"live":true,"active_run_id":1,"active_role":role}),
        );
        for (time, samples, busy) in [(50, 0, 0.0), (98, 480, 24.0)] {
            let row = json!({"worker_id":name,"created_at":DateTime::from_timestamp(time,0).unwrap(),"rss_bytes":1024,
                "metrics":{"epoch":"eval-epoch","node_uuid":name,"task_id":"2","samples_evaluated":samples,
                    "cumulative":{"evaluate_seconds":busy,"materialize_seconds":0.0},
                    "busy":{"elapsed_seconds":time,"compute_seconds":busy,"io_seconds":busy*0.5}},
                "runtime_metrics":{"runner_epoch":"sample-epoch","node_uuid":name,"task_id":"2",
                    "completed_samples_total":samples,
                    "busy":{"elapsed_seconds":time,"compute_seconds":busy,"io_seconds":busy*1.5},
                    "sampler":{"produce_ms_per_sample":{"count":1,"total":busy*500.0},
                        "completed_training_ingest_ms":{"count":1,"total":busy*500.0},
                        "completed_merge_ingest_ms":{"count":0,"total":null}}}});
            if role == "evaluator" {
                evaluators.push(row);
            } else {
                samplers.push(row);
            }
        }
    }
    snapshot.evaluators.push(evaluators[1].clone());
    snapshot.samplers.push(samplers[1].clone());
    (snapshot, evaluators, samplers)
}

fn overview(snapshot: &PerformanceSnapshot, e: &[Value], s: &[Value]) -> Value {
    let response = build_performance_response(snapshot, e, s, 60, None);
    let state = response
        .updates
        .iter()
        .find(|p| p.panel.panel_id() == "performance_overview")
        .unwrap();
    let value = serde_json::to_value(state).unwrap();
    let mut result = value["panel"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v["key"].as_str().unwrap().to_owned(), v["value"].clone()))
        .collect::<serde_json::Map<_, _>>();
    let busy = serde_json::to_value(
        &response
            .updates
            .iter()
            .find(|p| p.panel.panel_id() == "busy_rates")
            .unwrap()
            .panel,
    )
    .unwrap();
    result.insert("evaluator_busy".into(), busy["rows"][0][1].clone());
    result.insert("evaluator_io".into(), busy["rows"][0][2].clone());
    result.insert("sampler_busy".into(), busy["rows"][1][1].clone());
    result.insert("sampler_io".into(), busy["rows"][1][2].clone());
    result.into()
}

#[test]
fn overview_uses_live_identities_and_counter_intervals() {
    let (snapshot, e, s) = fixture();
    let result = overview(&snapshot, &e, &s);
    assert_eq!(result["accepted_rate"], 10.0);
    assert_eq!(result["sampler_busy"], 50.0);
    assert_eq!(result["evaluator_busy"], 50.0);
    assert_eq!(result["evaluator_io"], 25.0);
    assert_eq!(result["sampler_io"], 75.0);
    assert_eq!(result["allocated_core_hours"], 2.0);
    assert_eq!(result["coverage"], "2 / 2");
}

#[test]
fn stale_replaced_and_stopped_workers_do_not_supply_current_usage() {
    let (mut snapshot, e, s) = fixture();
    snapshot.nodes[0]["uuid"] = json!("replacement");
    let replaced = overview(&snapshot, &e, &s);
    assert!(replaced["evaluator_busy"].is_null());
    assert_eq!(replaced["coverage"], "1 / 2");
    assert_eq!(replaced["memory_coverage"], "1 / 2");
    snapshot.observed_at += chrono::Duration::seconds(20);
    let stale = overview(&snapshot, &e, &s);
    assert!(stale["accepted_rate"].is_null());
    assert!(stale["sampler_busy"].is_null());
    assert!(stale["process_rss"].is_null());
    snapshot
        .nodes
        .iter_mut()
        .for_each(|n| n["live"] = json!(false));
    assert_eq!(overview(&snapshot, &e, &s)["live_workers"], "0 / 0");
}

#[test]
fn rates_require_two_points_in_the_current_epoch_and_requested_window() {
    let (snapshot, e, mut s) = fixture();
    s[0]["runtime_metrics"]["runner_epoch"] = json!("old-epoch");
    assert!(overview(&snapshot, &e, &s)["accepted_rate"].is_null());
    s[0]["runtime_metrics"]["runner_epoch"] = json!("sample-epoch");
    s[0]["runtime_metrics"]["completed_samples_total"] = json!(1000);
    assert!(overview(&snapshot, &e, &s)["accepted_rate"].is_null());
    s[0]["runtime_metrics"]["completed_samples_total"] = json!(0);
    s[0]["created_at"] = json!(DateTime::from_timestamp(30, 0).unwrap());
    assert!(overview(&snapshot, &e, &s)["accepted_rate"].is_null());
}

#[test]
fn operation_means_use_totals_and_counts_instead_of_summing_conditional_means() {
    let mut panels = Vec::new();
    let mut states = Vec::new();
    let rows = [
        json!({"runtime_metrics":{"sampler":{"eval_ms_per_sample":{"count":1,"total":10.0}}}}),
        json!({"runtime_metrics":{"sampler":{"eval_ms_per_sample":{"count":1000,"total":100.0}}}}),
    ];
    add_sampler_timings(&mut panels, &mut states, &rows.iter().collect::<Vec<_>>());
    let value = serde_json::to_value(&states[0]).unwrap();
    let mean = value["rows"][0][4].as_f64().unwrap();
    assert!((mean - 110.0 / 1001.0).abs() < 1e-12);
    assert!(
        panels
            .iter()
            .all(|p| !matches!(p.kind, PanelKind::TickBreakdown))
    );
}

fn graph(snapshot: &PerformanceSnapshot, e: &[Value], s: &[Value], id: &str) -> Value {
    let end = snapshot.observed_at.timestamp_millis() as f64;
    let mut graphs = graphs::Graphs::new([end - 60000., end]);
    for (evaluator, rows) in [(true, e), (false, s)] {
        let mut rows = rows.to_vec();
        rows.sort_by_key(timestamp);
        for row in rows {
            graphs.observe(row, evaluator);
        }
    }
    let (_, states) = graphs.panels();
    serde_json::to_value(states.iter().find(|panel| panel.panel_id() == id).unwrap()).unwrap()
}

#[test]
fn cumulative_busy_survives_missing_publications_and_uses_monotonic_time() {
    let (snapshot, e, mut s) = fixture();
    // Database publication timestamps vary; the actual measurement clock does not.
    s[0]["created_at"] = json!(DateTime::from_timestamp(88, 0).unwrap());
    assert_eq!(overview(&snapshot, &e, &s)["sampler_busy"], 50.0);
    s[1]["runtime_metrics"]["busy"]["compute_seconds"] = json!(100.0);
    assert!(overview(&snapshot, &e, &s)["sampler_busy"].is_null());
    s[1]["runtime_metrics"]["busy"] = Value::Null;
    assert!(overview(&snapshot, &e, &s)["sampler_io"].is_null());
}

#[test]
fn graphs_keep_four_traces_and_completed_work_without_stale_current_values() {
    let (mut snapshot, e, s) = fixture();
    snapshot.task_id = None;
    snapshot.nodes.clear();
    assert!(overview(&snapshot, &e, &s)["sampler_busy"].is_null());
    let busy = graph(&snapshot, &e, &s, "busy_history");
    assert_eq!(busy["x_range"], json!([40000.0, 100000.0]));
    assert_eq!(busy["y_range"], json!([0.0, 100.0]));
    let series = busy["series"].as_array().unwrap();
    assert_eq!(series.len(), 4);
    for (line, expected) in series.iter().zip([50.0, 25.0, 50.0, 75.0]) {
        assert!(!line["points"].as_array().unwrap().is_empty());
        for point in line["points"].as_array().unwrap() {
            assert!((point["y"].as_f64().unwrap() - expected).abs() < 1e-9);
            assert!(point["x"].as_f64().unwrap() >= 50000.0);
        }
    }
}

#[test]
fn graphs_do_not_bridge_tasks_restarts_or_invalid_counters() {
    let (snapshot, mut e, s) = fixture();
    for key in ["task_id", "epoch", "node_uuid"] {
        let original = e[0]["metrics"][key].clone();
        e[0]["metrics"][key] = json!("previous");
        assert!(
            graph(&snapshot, &e, &s, "busy_history")["series"][0]["points"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        e[0]["metrics"][key] = original;
    }
    e[0]["metrics"]["samples_evaluated"] = json!(1000);
    assert!(
        graph(&snapshot, &e, &s, "busy_history")["series"][0]["points"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn missing_busy_is_unknown_while_reported_idle_is_zero() {
    let (snapshot, e, mut s) = fixture();
    s[0]["runtime_metrics"]["busy"] = Value::Null;
    let mut last = s[1].clone();
    last["created_at"] = json!(DateTime::from_timestamp(99, 0).unwrap());
    last["runtime_metrics"]["busy"]["elapsed_seconds"] = json!(99);
    s.push(last);
    let busy = graph(&snapshot, &e, &s, "busy_history");
    let points = busy["series"][2]["points"].as_array().unwrap();
    assert!(!points.is_empty());
    assert!(points.iter().all(|point| point["y"] == 0.0));
    assert_eq!(points[0]["x"], 98000.0);
    assert_eq!(points[0]["y"], 0.0);
    assert_eq!(points[0]["break_before"], true);
}

#[test]
fn evaluator_fleet_is_time_weighted_and_requires_complete_current_coverage() {
    let (mut snapshot, mut e, s) = fixture();
    let mut node = snapshot.nodes[0].clone();
    node["name"] = json!("e2");
    node["uuid"] = json!("e2");
    snapshot.nodes.push(node);
    let mut extra = e.clone();
    for row in &mut extra {
        row["worker_id"] = json!("e2");
        row["metrics"]["node_uuid"] = json!("e2");
    }
    extra[1]["metrics"]["busy"] =
        json!({"elapsed_seconds":74.0,"compute_seconds":24.0,"io_seconds":24.0});
    snapshot.evaluators.push(extra[1].clone());
    e.extend(extra);
    let value = overview(&snapshot, &e, &s);
    assert!((value["evaluator_busy"].as_f64().unwrap() - 100.0 * 48.0 / 72.0).abs() < 1e-9);
    assert_eq!(value["evaluator_io"], 50.0);
    e.last_mut().unwrap()["metrics"]["busy"] = Value::Null;
    assert!(overview(&snapshot, &e, &s)["evaluator_io"].is_null());
}
