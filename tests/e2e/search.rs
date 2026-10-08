use super::*;

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn search_controllers() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;
    harness.start_nodes(&["s", "e0", "e1"]).await?;
    for mode in ["scan", "grid", "random", "egobox", "failed-measurement"] {
        let scan = mode == "scan";
        let failed = mode == "failed-measurement";
        let kind = if scan {
            "parameter_scan"
        } else {
            "hyperparameter_tuning"
        };
        let controller = match mode {
            "scan" => r#"
max_concurrent_runs = 2
[[parameters]]
name = "bins"
values = [8,12,16]
[[parameters]]
name = "mode"
values = ["auto","none"]
[measurement]
source_task = "sample"
"#
            .to_string(),
            _ => {
                let (algorithm, params, parameters) = match mode {
                    "grid" => ("grid_search", "", ""),
                    "egobox" => (
                        "egobox",
                        "max_trials=3\nseed=3\ninitial_design=2\nparallel_candidates=1\ninfill='ei'",
                        "",
                    ),
                    "random" => (
                        "random_search",
                        "max_trials=4\nseed=3",
                        "[parameters.a]\nkind='float'\nmin=0.0\nmax=1.0",
                    ),
                    _ => ("random_search", "max_trials=1\nseed=5", ""),
                };
                format!(
                    r#"
max_concurrent_trials = 2
[optimizer]
algorithm = "{algorithm}"
[optimizer.params]
{params}
[objective]
source_task = "sample"
mode = "minimize"
quantity = "central_value"
[parameters.bins]
kind = "integer"
min = 8
max = 16
step = 4
[parameters.mode]
kind = "categorical"
values = ["auto","none"]
{parameters}
"#
                )
            }
        };
        let measurement = if failed {
            "{ metric = 'variance', component = 'missing' }"
        } else {
            "'central_value'"
        };
        let name = format!("search-{mode}");
        harness.add_run(&temp_config(&format!(
            r#"
name = "{name}"
kind = "{kind}"
{controller}
[child]
run = '''
name = "{mode}-bins-$(bins:8)-mode-$(mode:auto)-a-$(a:0.0)"
[evaluator]
kind = "unit"
timing = {{ per_sample_seconds = 0.002 }}
[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = {{ max_samples = 64 }}
measurement = {{ quantity = {measurement} }}
accumulator = {{ config = "scalar" }}
sampler_aggregator = {{ config = {{ kind = "naive_monte_carlo" }} }}
'''
"#
        )));
        let run = harness.run_id(&name).await?;
        for (node, role) in [
            ("s", "sampler_aggregator"),
            ("e0", "evaluator"),
            ("e1", "evaluator"),
        ] {
            harness.assign_node(node, role, &name);
        }
        let count_key = if scan {
            "completed_points"
        } else {
            "completed_trials"
        };
        let saw_partial = std::cell::Cell::new(false);
        let saw_redistributed = std::cell::Cell::new(false);
        let pool = &harness.pool;
        harness.wait_for(format!("{mode} controller completes"),Duration::from_secs(60),|| {
            // Inspect progress in the same loop as completion, avoiding a second
            // race-prone wait for a transient state after it may have passed.
            async {
                let (state, reason, output): (String,Option<String>,Option<JsonValue>) = sqlx::query_as(
                    "SELECT state,failure_reason,controller_output FROM run_tasks WHERE run_id=$1")
                    .bind(run).fetch_one(pool).await?;
                anyhow::ensure!(failed || state!="failed", "{mode}: {reason:?}");
                if let Some(output) = output {
                    let n = output[count_key].as_i64().unwrap_or(0);
                    let total = output[if scan { "total_points" } else { "total_trials" }].as_i64().unwrap_or(0);
                    saw_partial.set(saw_partial.get() || (n>0 && n<total));
                }
                saw_redistributed.set(saw_redistributed.get() || sqlx::query_scalar::<_,bool>(
                    "SELECT EXISTS(SELECT 1 FROM nodes n JOIN runs r ON r.id=n.desired_run_id WHERE r.parent_run_id=$1)")
                    .bind(run).fetch_one(pool).await?);
                Ok(state == if failed { "failed" } else { "completed" })
            }
        }).await?;
        anyhow::ensure!(
            saw_redistributed.get(),
            "{mode}: parent workers never reached children"
        );
        let output: JsonValue =
            sqlx::query_scalar("SELECT controller_output FROM run_tasks WHERE run_id=$1")
                .bind(run)
                .fetch_one(&harness.pool)
                .await?;
        let items = output[if scan { "points" } else { "trials" }]
            .as_array()
            .unwrap();
        let expected = match mode {
            "scan" | "grid" => 6,
            "random" => 4,
            "egobox" => 3,
            _ => 1,
        };
        assert_eq!(items.len(), expected, "{mode}");
        if failed {
            assert_eq!(output["failed_trials"], 1);
            assert_eq!(output[count_key], 0);
            let reason = items[0]["failure_reason"].as_str().unwrap();
            for part in [
                "child_run_id=",
                "source_task=sample",
                "requested=Mean",
                "Variance(component=missing)",
                "measurement failed",
                "unavailable",
            ] {
                anyhow::ensure!(
                    reason.contains(part),
                    "incomplete error provenance: {reason}"
                );
            }
        } else {
            anyhow::ensure!(
                saw_partial.get(),
                "{mode}: no intermediate controller progress observed"
            );
            assert_eq!(output[count_key], expected);
            if !scan {
                assert_eq!(output["failed_trials"], 0);
                assert!(output["best_trial"].is_number());
                assert!(output["best_result_source"]["snapshot_id"].is_string());
            }
            for (i, item) in items.iter().enumerate() {
                assert!(item["result_source"]["snapshot_id"].is_string());
                let values = &item[if scan {
                    "parameter_values"
                } else {
                    "parameters"
                }];
                if mode == "scan" || mode == "grid" {
                    assert_eq!(
                        *values,
                        json!({"bins":8+4*(i/2),"mode":if i%2==0 {"auto"} else {"none"}})
                    );
                } else {
                    assert!([8, 12, 16].contains(&values["bins"].as_i64().unwrap()));
                    assert!(item["objective_value"].is_number());
                }
            }
            let measured: i64 = sqlx::query_scalar("SELECT count(*) FROM runs r JOIN run_tasks t ON t.run_id=r.id WHERE r.parent_run_id=$1 AND t.name='sample' AND t.measurement_output IS NOT NULL")
                .bind(run).fetch_one(&harness.pool).await?;
            assert_eq!(measured, expected as i64);
            assert_fresh_controller_duplicate(&harness, &name).await?;
        }
        let children: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE parent_run_id=$1")
            .bind(run)
            .fetch_one(&harness.pool)
            .await?;
        assert_eq!(children, expected as i64);
    }
    harness.cleanup().await
}
