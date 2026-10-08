use super::*;

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_cli_lists_duplicate_run_names_and_reports_ambiguity() -> anyhow::Result<()> {
    let mut harness = FullStackHarness::new().await?;

    let config_a = temp_config("name = \"duplicate-run\"\n");
    let config_b = temp_config("name = \"duplicate-run\"\n");

    harness.add_run(&config_a);
    harness.add_run(&config_b);

    let rows = sqlx::query("SELECT id FROM runs WHERE name = 'duplicate-run' ORDER BY id ASC")
        .fetch_all(&harness.pool)
        .await?;
    assert_eq!(rows.len(), 2);
    let id_a: i32 = rows[0].try_get("id")?;
    let id_b: i32 = rows[1].try_get("id")?;

    let list_output = harness
        .cli()
        .args(["run", "list", "duplicate-run"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let list_output = String::from_utf8(list_output)?;
    assert!(list_output.contains("duplicate-run"));
    assert!(list_output.contains(&id_a.to_string()));
    assert!(list_output.contains(&id_b.to_string()));

    harness
        .cli()
        .args(["run", "pause", "duplicate-run"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "run name 'duplicate-run' matches multiple runs",
        ))
        .stderr(predicate::str::contains(format!("id={id_a}")))
        .stderr(predicate::str::contains(format!("id={id_b}")));

    harness.cleanup().await?;
    Ok(())
}

#[test]
fn orchestration_cannot_be_queued_as_integration_tasks() {
    let error = gammaboard::api::runs::parse_run_add_config_toml(
        r#"
name = "invalid-chain"
[[task_queue]]
kind = "parameter_scan"
"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("orchestration is a run kind"));
}

#[test]
fn tuning_rejects_mixed_file_and_inline_child_definitions() {
    let error = gammaboard::api::runs::parse_run_add_config_toml(
        r#"
kind = "hyperparameter_tuning"
name = "invalid-child"
[child]
run = { file = "child.toml", name = "inline-too" }
"#,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("TOML string or a file reference")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_definition_editor_duplicates_current_drafts() -> anyhow::Result<()> {
    use gammaboard::api::runs;
    use gammaboard::core::{RunTaskState, RunTaskStore};

    let mut harness = FullStackHarness::new().await?;
    let raw = r#"
name = "editor-source"
[evaluator]
kind = "unit"
continuous_dims = 1
[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 16 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"#;
    harness.add_run(&temp_config(raw));
    let run_id = harness.run_id("editor-source").await?;
    let store = gammaboard::PgStore::new(harness.pool.clone());
    let server_url = harness.start_server().await?;
    let cookie = login_cookie(&server_url).await?;
    let draft = runs::export_run_definition(&store, run_id)
        .await?
        .replace("max_samples = 16", "max_samples = 123");
    for name in ["editor-source-copy", "editor-source-copy-2", "custom-name"] {
        let draft = if name == "custom-name" {
            runs::rename_run_definition(&draft, name)?
        } else {
            draft.clone()
        };
        let response = http_post_json(
            &server_url,
            "/api/runs",
            json!({"toml": draft, "duplicate": true}),
            Some(&cookie),
        )
        .await?
        .error_for_status()?
        .text()
        .await?;
        let response: JsonValue = serde_json::from_str(&response)?;
        assert_eq!(response["run_name"], name);
        let id = response["run_id"].as_i64().unwrap() as i32;
        let tasks = store.list_run_tasks(id).await?;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].state, RunTaskState::Pending);
        assert_eq!(tasks[0].nr_completed_samples, 0);
        assert_eq!(
            serde_json::to_value(&tasks[0].task)?["stop_condition"]["max_samples"],
            123
        );
        let exported: toml::Value =
            toml::from_str(&runs::export_run_definition(&store, id).await?)?;
        assert_eq!(exported["name"].as_str(), Some(name));
    }

    let source = store.list_run_tasks(run_id).await?;
    let draft = runs::export_task_definition(&store, run_id, source[0].id, false)
        .await?
        .replace("max_samples = 16", "max_samples = 321");
    for name in ["sample-copy", "sample-copy-2", "custom-task"] {
        let draft = if name == "custom-task" {
            draft.replace("name = \"sample\"", "name = \"custom-task\"")
        } else {
            draft.clone()
        };
        let response = http_post_json(
            &server_url,
            &format!("/api/runs/{run_id}/tasks"),
            json!({"toml": draft, "duplicate": true}),
            Some(&cookie),
        )
        .await?
        .error_for_status()?
        .text()
        .await?;
        let response: JsonValue = serde_json::from_str(&response)?;
        assert_eq!(response[0]["name"], name);
    }
    let tasks = store.list_run_tasks(run_id).await?;
    assert_eq!(tasks.len(), 4);
    assert_eq!(
        serde_json::to_value(&tasks[0].task)?["stop_condition"]["max_samples"],
        16
    );
    for task in &tasks[1..] {
        assert_eq!(task.state, RunTaskState::Pending);
        assert_eq!(
            serde_json::to_value(&task.task)?["stop_condition"]["max_samples"],
            321
        );
    }
    let queued = draft
        .replace("[task]", "[[task_queue]]")
        .replace("[task.", "[task_queue.");
    let two_tasks = format!("{queued}\n{queued}");
    let rejected = http_post_json(
        &server_url,
        &format!("/api/runs/{run_id}/tasks"),
        json!({"toml": two_tasks, "duplicate": true}),
        Some(&cookie),
    )
    .await?;
    assert_eq!(rejected.status(), reqwest::StatusCode::BAD_REQUEST);
    assert!(rejected.text().await?.contains("exactly one task"));
    assert_eq!(store.list_run_tasks(run_id).await?.len(), 4);

    // Controller exports read the frozen effective document, whose name must also change.
    let controller = format!(
        r#"
kind = "parameter_scan"
name = "editor-scan"
max_concurrent_runs = 1
[[parameters]]
name = "scale"
values = [1]
[measurement]
source_task = "sample"
[child]
run = '''{raw}'''
"#
    );
    harness.add_run(&temp_config(&controller));
    let draft = controller.replace("values = [1]", "values = [1, 2]");
    let response = http_post_json(
        &server_url,
        "/api/runs",
        json!({"toml": draft, "duplicate": true}),
        Some(&cookie),
    )
    .await?
    .error_for_status()?
    .text()
    .await?;
    let response: JsonValue = serde_json::from_str(&response)?;
    assert_eq!(response["run_name"], "editor-scan-copy");
    let id = response["run_id"].as_i64().unwrap() as i32;
    let exported: toml::Value = toml::from_str(&runs::export_run_definition(&store, id).await?)?;
    assert_eq!(exported["name"].as_str(), Some("editor-scan-copy"));
    assert_eq!(
        exported["parameters"][0]["values"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let children: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE parent_run_id=$1")
        .bind(id)
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(children, 0);
    harness.cleanup().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_definition_duplication_and_pending_task_edits() -> anyhow::Result<()> {
    use gammaboard::api::runs;
    use gammaboard::core::{RunTaskState, RunTaskStore, traits::TaskQueueChange};
    let mut harness = FullStackHarness::new().await?;
    let config = temp_config(
        r#"
name = "definition-source"
[evaluator]
kind = "unit"
continuous_dims = 1
[[task_queue]]
name = "setup"
kind = "set_accumulator"
accumulator = "scalar"
[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 16 }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
[[task_queue]]
name = "later"
kind = "sample"
stop_condition = { max_samples = 16 }
accumulator = { from_name = "sample" }
"#,
    );
    harness.add_run(&config);
    let run_id = harness.run_id("definition-source").await?;
    let store = gammaboard::PgStore::new(harness.pool.clone());
    let tasks = store.list_run_tasks(run_id).await?;
    let sample_id = tasks[1].id;
    let server_url = harness.start_server().await?;
    let cookie = login_cookie(&server_url).await?;
    let definition = http_get_with_cookie(
        &server_url,
        &format!("/api/runs/{run_id}/tasks/{sample_id}/definition"),
        &cookie,
    )
    .await?;
    let definition: JsonValue = serde_json::from_str(&definition)?;
    let raw = definition["toml"].as_str().unwrap().to_owned();
    let edited = raw.replace("max_samples = 16", "max_samples = 24");
    let response = reqwest::Client::new()
        .put(format!("{server_url}/api/runs/{run_id}/tasks/{sample_id}"))
        .header("cookie", &cookie)
        .header("content-type", "application/json")
        .body(json!({"toml": edited, "expected_toml": raw}).to_string())
        .send()
        .await?;
    let status = response.status();
    assert!(
        status.is_success(),
        "edit response {status}: {}",
        response.text().await?
    );
    assert!(
        runs::edit_pending_task(&store, run_id, sample_id, &raw, &raw)
            .await
            .is_err(),
        "stale editor must not overwrite an edit"
    );
    let renamed = edited.replace("name = \"sample\"", "name = \"renamed\"");
    assert!(
        runs::edit_pending_task(&store, run_id, sample_id, &renamed, &edited)
            .await
            .is_err(),
        "later named sources must stay valid"
    );
    assert!(
        runs::remove_pending_task(&store, run_id, sample_id)
            .await
            .is_err()
    );
    assert!(
        store
            .apply_task_queue_change(
                run_id,
                &tasks,
                TaskQueueChange::Remove { task_id: sample_id }
            )
            .await
            .is_err(),
        "optimistic writes must reject changed queues"
    );
    harness
        .cli()
        .args([
            "run",
            "task",
            "duplicate",
            "definition-source",
            &sample_id.to_string(),
        ])
        .assert()
        .success();
    let tasks = store.list_run_tasks(run_id).await?;
    assert_eq!(tasks.len(), 4);
    assert_eq!(tasks[3].name, "sample-copy");
    assert_eq!(tasks[3].state, RunTaskState::Pending);
    // Export includes all definitions, even when execution states differ.
    sqlx::query("UPDATE run_tasks SET state='completed' WHERE id=$1")
        .bind(tasks[0].id)
        .execute(&harness.pool)
        .await?;
    sqlx::query("UPDATE run_tasks SET state='failed' WHERE id=$1")
        .bind(tasks[1].id)
        .execute(&harness.pool)
        .await?;
    assert!(
        runs::edit_pending_task(&store, run_id, sample_id, &edited, &edited)
            .await
            .is_err()
    );
    harness
        .cli()
        .args(["run", "duplicate", "definition-source", "definition-copy"])
        .assert()
        .success();
    let copy_id = harness.run_id("definition-copy").await?;
    let copied = store.list_run_tasks(copy_id).await?;
    assert_eq!(copied.len(), 4);
    assert!(
        copied
            .iter()
            .all(|task| task.state == RunTaskState::Pending && task.nr_completed_samples == 0)
    );
    let inherited: i64 = sqlx::query_scalar("SELECT count(*) FROM run_stage_snapshots WHERE run_id=$1 AND (sampler_snapshot IS NOT NULL OR observable_state IS NOT NULL)")
        .bind(copy_id).fetch_one(&harness.pool).await?;
    assert_eq!(inherited, 0);
    // Live tuning must retain other fields saved by an overlapping definition edit.
    let tune_id = copied[3].id;
    let mut task_json = serde_json::to_value(&copied[3].task)?;
    task_json["stop_condition"]["max_samples"] = json!(123);
    let task_toml = runs::export_task_definition(&store, copy_id, tune_id, false)
        .await?
        .replace("max_samples = 24", "max_samples = 123");
    let mut editing = harness.pool.begin().await?;
    sqlx::query("UPDATE run_tasks SET task=$2, task_toml=$3 WHERE id=$1")
        .bind(tune_id)
        .bind(task_json)
        .bind(task_toml)
        .execute(&mut *editing)
        .await?;
    let tuning_store = store.clone();
    let mut tuning = tokio::spawn(async move {
        tuning_store
            .update_run_task_queue_tuning(copy_id, tune_id, None)
            .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut tuning)
            .await
            .is_err()
    );
    editing.commit().await?;
    let tuned = tuning.await??;
    assert_eq!(
        serde_json::to_value(tuned.task)?["stop_condition"]["max_samples"],
        json!(123)
    );
    // Edits reject activation races, even when definitions are unchanged.
    let expected = store.list_run_tasks(copy_id).await?;
    // A queue edit must not make activation skip a task or report an exhausted queue.
    let mut editing = harness.pool.begin().await?;
    sqlx::query("SELECT id FROM run_tasks WHERE id=$1 FOR UPDATE")
        .bind(expected[0].id)
        .fetch_one(&mut *editing)
        .await?;
    let activation_store = store.clone();
    let mut activation =
        tokio::spawn(async move { activation_store.activate_next_run_task(copy_id).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut activation)
            .await
            .is_err()
    );
    editing.commit().await?;
    let active = activation.await??.unwrap();
    assert_eq!(active.id, expected[0].id);
    assert!(
        store
            .apply_task_queue_change(
                copy_id,
                &expected,
                TaskQueueChange::Remove { task_id: active.id }
            )
            .await
            .is_err()
    );
    let response = harness
        .cli()
        .args(["run", "export", "definition-copy"])
        .output()?;
    anyhow::ensure!(response.status.success());
    let exported = runs::parse_run_add_config_toml(std::str::from_utf8(&response.stdout)?)?;
    assert_eq!(exported.task_queue.unwrap().len(), 4);
    harness.cleanup().await
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn worker_resume_is_durable_backend_neutral_and_consumed_once() -> anyhow::Result<()> {
    use gammaboard::core::ControlPlaneStore;
    let db = TestDatabase::create().await?;
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&db.database_url)
        .await?;
    let store = gammaboard::PgStore::new(pool.clone());
    let group = serde_json::json!({"count":2,"name_prefix":"resume","max_start_failures":7,"config":{"partition":"gpu","cpus":4,"gres":"gpu:a100:1"}});
    let id = store
        .reserve_worker_launch_with_args("external", vec![group], json!({"partition":"epyc2"}))
        .await?;
    let config = temp_config(
        r#"
name = "resume-assignment"
[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0
[[task_queue]]
kind = "sample"
stop_condition = { max_samples=100 }
accumulator = { config="scalar" }
sampler_aggregator = { config={kind="naive_monte_carlo"} }
"#,
    );
    let config = gammaboard::api::runs::load_run_add_config_file(config.path())?;
    let run = gammaboard::api::runs::create_run(&store, config).await?;

    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM nodes ORDER BY name")
        .fetch_all(&pool)
        .await?;
    store
        .announce_node(&names[0], "first", &Default::default())
        .await?;
    store
        .assign_worker_pool(
            &names[0],
            gammaboard::core::WorkerRole::SamplerAggregator,
            run.run_id,
        )
        .await?;
    assert_eq!(store.suspend_workers().await?, 1); // Pending scheduler jobs are not saved.
    store.expire_node_lease("first").await?;
    let saved: bool = sqlx::query_scalar("SELECT resume_requested FROM nodes WHERE name=$1")
        .bind(&names[0])
        .fetch_one(&pool)
        .await?;
    assert!(saved);
    assert_eq!(store.enqueue_resumed_workers().await?, 1);
    assert_eq!(store.enqueue_resumed_workers().await?, 0);
    let requests = store.list_node_launch_requests().await?;
    let resumed = requests.iter().find(|r| r.id != id).unwrap();
    assert_eq!(resumed.backend, "external");
    assert_eq!(resumed.args["partition"], "epyc2");
    assert_eq!(resumed.args["groups"][0]["config"]["partition"], "gpu");
    assert_eq!(resumed.args["groups"][0]["max_start_failures"], 7);
    assert_eq!(resumed.args["groups"][0]["node_names"][0], names[0]);
    assert!(store.claim_local_worker_launch().await?.is_none());
    store
        .announce_node(&names[0], "second", &Default::default())
        .await?;
    assert_eq!(
        store
            .get_desired_assignment(&names[0])
            .await?
            .unwrap()
            .run_id,
        run.run_id
    );
    store.suspend_workers().await?;
    store.expire_node_lease("second").await?;
    // An operator pause while the fleet is down still clears the saved assignment.
    store.clear_desired_assignments_for_run(run.run_id).await?;
    assert!(store.get_desired_assignment(&names[0]).await?.is_none());
    store.request_node_shutdown(&names[0]).await?;
    assert_eq!(store.enqueue_resumed_workers().await?, 0);
    pool.close().await;
    db.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn controller_assignments_are_stable_atomic_and_reject_stale_plans() -> anyhow::Result<()> {
    use gammaboard::core::{
        ControlPlaneStore, DesiredAssignment, NodeAssignmentUpdate, WorkerRole,
    };
    use gammaboard::runners::controller_child::{
        ControllerAssignmentPlan, apply_controller_assignment_plan,
    };
    use gammaboard::stores::PgStore;
    let mut harness = FullStackHarness::new().await?;
    for name in ["parent", "left", "right", "unrelated"] {
        harness.add_run(&temp_config(&format!("name = '{name}'")));
    }
    let parent = harness.run_id("parent").await?;
    let left = harness.run_id("left").await?;
    let right = harness.run_id("right").await?;
    let unrelated = harness.run_id("unrelated").await?;
    sqlx::query("UPDATE runs SET parent_run_id=$1 WHERE id IN ($2,$3)")
        .bind(parent)
        .bind(left)
        .bind(right)
        .execute(&harness.pool)
        .await?;
    sqlx::query("UPDATE runs SET integration_params=jsonb_set(integration_params, '{run_kind}', '\"parameter_scan\"') WHERE id=$1")
        .bind(parent).execute(&harness.pool).await?;
    let store = PgStore::new(harness.pool.clone());
    for (name, role, run_id) in [
        ("s", WorkerRole::SamplerAggregator, parent),
        ("e", WorkerRole::Evaluator, parent),
        ("other", WorkerRole::Evaluator, unrelated),
    ] {
        store.announce_node(name, name, &Default::default()).await?;
        store.assign_worker_pool(name, role, run_id).await?;
    }
    sqlx::query("UPDATE nodes SET lease_expires_at = now() + interval '5 minutes'")
        .execute(&harness.pool)
        .await?;
    let plan = |selected| ControllerAssignmentPlan::new(parent, vec![selected]);
    apply_controller_assignment_plan(&store, plan(left)).await?;
    let versions: Vec<(String, String)> =
        sqlx::query_as("SELECT name, xmin::text FROM nodes ORDER BY name")
            .fetch_all(&harness.pool)
            .await?;
    for _ in 0..20 {
        apply_controller_assignment_plan(&store, plan(left)).await?;
    }
    assert_eq!(
        versions,
        sqlx::query_as::<_, (String, String)>("SELECT name, xmin::text FROM nodes ORDER BY name")
            .fetch_all(&harness.pool)
            .await?,
        "unchanged ticks must not rewrite assignments"
    );
    // Deliberately widen the old clear/restore race. Concurrent readers must
    // still see one complete pool, never unassigned or partly moved workers.
    sqlx::raw_sql(
        "CREATE FUNCTION delay_assignment_clear() RETURNS trigger LANGUAGE plpgsql AS $$
        BEGIN IF OLD.desired_run_id IS NOT NULL AND NEW.desired_run_id IS NULL THEN
            PERFORM pg_sleep(0.02); END IF; RETURN NEW; END $$;
        CREATE TRIGGER delay_assignment_clear BEFORE UPDATE ON nodes
        FOR EACH ROW EXECUTE FUNCTION delay_assignment_clear();",
    )
    .execute(&harness.pool)
    .await?;
    let writer_store = store.clone();
    let writer = tokio::spawn(async move {
        for _ in 0..5 {
            for selected in [right, left] {
                apply_controller_assignment_plan(
                    &writer_store,
                    ControllerAssignmentPlan::new(parent, vec![selected]),
                )
                .await?;
            }
        }
        Ok::<_, anyhow::Error>(())
    });
    let mut reads = 0;
    while !writer.is_finished() {
        let assignments: Vec<Option<i32>> = sqlx::query_scalar(
            "SELECT desired_run_id FROM nodes WHERE name IN ('s', 'e') ORDER BY name",
        )
        .fetch_all(&harness.pool)
        .await?;
        assert!(
            assignments == vec![Some(left), Some(left)]
                || assignments == vec![Some(right), Some(right)],
            "workers observed a partial assignment: {assignments:?}"
        );
        reads += 1;
        sleep(Duration::from_millis(2)).await;
    }
    writer.await??;
    assert!(reads > 0);
    assert_eq!(
        store.get_desired_assignment("other").await?.unwrap().run_id,
        unrelated
    );
    let updates = store
        .list_nodes(None)
        .await?
        .into_iter()
        .filter(|n| n.name != "other")
        .map(|n| NodeAssignmentUpdate {
            node_uuid: n.uuid,
            expected: n.desired_assignment.clone(),
            desired: Some(DesiredAssignment {
                run_id: right,
                ..n.desired_assignment.unwrap()
            }),
        })
        .collect::<Vec<_>>();
    // Concurrent manual reassignment invalidates the entire plan, including its
    // otherwise valid sampler change.
    store
        .assign_worker_pool("e", WorkerRole::Evaluator, unrelated)
        .await?;
    assert!(!store.update_desired_assignments(&updates).await?);
    assert_eq!(
        store.get_desired_assignment("s").await?.unwrap().run_id,
        left
    );
    assert_eq!(
        store.get_desired_assignment("e").await?.unwrap().run_id,
        unrelated
    );
    store
        .assign_worker_pool("e", WorkerRole::Evaluator, left)
        .await?;
    sqlx::query("UPDATE nodes SET lease_expires_at = now() - interval '1 second' WHERE name = 's'")
        .execute(&harness.pool)
        .await?;
    assert!(
        !store.update_desired_assignments(&updates).await?,
        "expired node must invalidate the plan"
    );
    store
        .announce_node("s", "replacement-s", &Default::default())
        .await?;
    assert!(
        !store.update_desired_assignments(&updates).await?,
        "new node UUID must invalidate the plan"
    );
    assert_eq!(
        store.get_desired_assignment("e").await?.unwrap().run_id,
        parent
    );
    store
        .assign_worker_pool("s", WorkerRole::SamplerAggregator, left)
        .await?;
    sqlx::query("UPDATE nodes SET lease_expires_at = now() - interval '1 second' WHERE name = 's'")
        .execute(&harness.pool)
        .await?;
    store
        .announce_node("new-s", "new-s", &Default::default())
        .await?;
    assert!(
        store
            .update_desired_assignments(&[NodeAssignmentUpdate {
                node_uuid: "new-s".into(),
                expected: None,
                desired: Some(DesiredAssignment {
                    node_name: "new-s".into(),
                    run_id: left,
                    role: WorkerRole::SamplerAggregator,
                    run_name: None,
                }),
            }])
            .await?,
        "an expired sampler must not block its differently named replacement"
    );
    harness.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn full_stack_child_creation_is_atomic_and_idempotent() -> anyhow::Result<()> {
    use gammaboard::api::runs::{ChildRunRequest, create_child_run};
    use gammaboard::core::tasks::ChildRunSource;
    let mut harness = FullStackHarness::new().await?;
    let child = "name='atomic-child'\n[evaluator]\nkind='unit'\ncontinuous_dims=1\n[[task_queue]]\nkind='sample'\nstop_condition={max_samples=100}\nmeasurement={quantity='central_value'}\naccumulator={config='scalar'}\nsampler_aggregator={config={kind='naive_monte_carlo'}}\n";
    let parent = temp_config(&format!(
        "kind='integration_campaign'\nname='atomic-parent'\nstop_condition={{ max_total_samples=100 }}\n[[children]]\nname='a'\nrun='''{child}'''\n"
    ));
    harness.add_run(&parent);
    let parent_id = harness.run_id("atomic-parent").await?;
    let task_id: i64 = sqlx::query_scalar("SELECT id FROM run_tasks WHERE run_id=$1")
        .bind(parent_id)
        .fetch_one(&harness.pool)
        .await?;
    let store = gammaboard::PgStore::new(harness.pool.clone());
    let request = ChildRunRequest {
        parent_run_id: parent_id,
        parent_task_id: Some(task_id),
        spawn_kind: "integration_campaign".into(),
        spawn_label: Some("a".into()),
        run: ChildRunSource::Inline(child.into()),
        replacements: Default::default(),
    };
    let (a, b) = tokio::join!(
        create_child_run(&store, request.clone()),
        create_child_run(&store, request.clone())
    );
    assert_eq!(a?.run_id, b?.run_id);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(count, 2, "concurrent retry creates one parented child");
    let bad = ChildRunRequest {
        parent_run_id: i32::MAX,
        spawn_label: Some("bad-parent".into()),
        ..request
    };
    assert!(create_child_run(&store, bad).await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&harness.pool)
        .await?;
    assert_eq!(count, 2, "failed parenting must not leave an orphan");
    harness.cleanup().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local postgres with CREATE DATABASE privilege"]
async fn worker_pool_operations_resolve_children_and_preserve_operator_intent() -> anyhow::Result<()>
{
    use gammaboard::api::{nodes, runs};
    use gammaboard::core::{ControlPlaneStore, RunReadStore, WorkerRole};
    use gammaboard::runners::controller_child::{
        ControllerAssignmentPlan, apply_controller_assignment_plan,
    };
    use gammaboard::stores::PgStore;
    for kind in [
        "integration_campaign",
        "parameter_scan",
        "hyperparameter_tuning",
    ] {
        let mut harness = FullStackHarness::new().await?;
        for name in ["root", "left", "right", "grandchild", "outside"] {
            harness.add_run(&temp_config(&format!("name = '{name}'")));
        }
        let root = harness.run_id("root").await?;
        let left = harness.run_id("left").await?;
        let right = harness.run_id("right").await?;
        let grandchild = harness.run_id("grandchild").await?;
        let outside = harness.run_id("outside").await?;
        sqlx::query("UPDATE runs SET integration_params=jsonb_set(integration_params, '{run_kind}', to_jsonb($2::text)) WHERE id=$1")
            .bind(root).bind(kind).execute(&harness.pool).await?;
        sqlx::query("UPDATE runs SET parent_run_id=$1 WHERE id IN ($2,$3)")
            .bind(root)
            .bind(left)
            .bind(right)
            .execute(&harness.pool)
            .await?;
        sqlx::query("UPDATE runs SET parent_run_id=$1 WHERE id=$2")
            .bind(left)
            .bind(grandchild)
            .execute(&harness.pool)
            .await?;
        let store = PgStore::new(harness.pool.clone());
        for name in ["s1", "s2", "s3", "e1", "e2", "idle"] {
            store.announce_node(name, name, &Default::default()).await?;
        }
        sqlx::query("UPDATE nodes SET lease_expires_at=now()+interval '5 minutes'")
            .execute(&harness.pool)
            .await?;
        for name in ["s1", "s2", "s3", "e1", "e2"] {
            let role = if name.starts_with('s') {
                WorkerRole::SamplerAggregator
            } else {
                WorkerRole::Evaluator
            };
            let response = nodes::assign_node(&store, name, grandchild, role).await?;
            assert_eq!(
                response.run_id, root,
                "{kind}: public response must identify owner"
            );
        }
        // Multiple sampler members can wait at a controller without colliding.
        apply_controller_assignment_plan(
            &store,
            ControllerAssignmentPlan::new(root, vec![left, right]),
        )
        .await?;
        assert_eq!(
            store.get_desired_assignment("s1").await?.unwrap().run_id,
            left
        );
        assert_eq!(
            store.get_desired_assignment("s2").await?.unwrap().run_id,
            right
        );
        assert_eq!(
            store.get_desired_assignment("s3").await?.unwrap().run_id,
            root
        );
        // A sibling request changes no placement and cannot pin or steal a slot.
        nodes::assign_node(&store, "s1", right, WorkerRole::SamplerAggregator).await?;
        assert_eq!(
            store.get_desired_assignment("s1").await?.unwrap().run_id,
            left
        );
        // Nested controller owns its branch's placement; root ticks leave it alone.
        apply_controller_assignment_plan(
            &store,
            ControllerAssignmentPlan::new(left, vec![grandchild]),
        )
        .await?;
        apply_controller_assignment_plan(
            &store,
            ControllerAssignmentPlan::new(root, vec![left, right]),
        )
        .await?;
        assert_eq!(
            store.get_desired_assignment("s1").await?.unwrap().run_id,
            grandchild
        );
        // A root selection change reclaims capacity even from deep descendants.
        apply_controller_assignment_plan(&store, ControllerAssignmentPlan::new(root, vec![right]))
            .await?;
        assert_ne!(
            store.get_desired_assignment("s1").await?.unwrap().run_id,
            grandchild
        );
        let paused = runs::pause_run(&store, grandchild).await?;
        assert_eq!(paused.run_id, root);
        assert_eq!(paused.assignments_cleared, 5);
        apply_controller_assignment_plan(&store, ControllerAssignmentPlan::new(root, vec![right]))
            .await?;
        assert!(store.list_desired_assignments(None).await?.is_empty());
        let members = store.get_registered_workers(Some(root)).await?;
        assert_eq!(
            members.len(),
            5,
            "paused members must remain visible on the parent"
        );
        assert!(members.iter().all(|n| n.pool_run_id == Some(root)));
        nodes::unassign_node(&store, "e1").await?;
        nodes::assign_node(&store, "e2", outside, WorkerRole::Evaluator).await?;
        let resumed = nodes::auto_assign_run(&store, grandchild, Some(0)).await?;
        assert_eq!(resumed.run_id, root);
        assert_eq!(resumed.resumed_nodes, 3);
        assert!(resumed.sampler_already_assigned);
        apply_controller_assignment_plan(&store, ControllerAssignmentPlan::new(root, vec![left]))
            .await?;
        assert!(store.get_desired_assignment("e1").await?.is_none());
        assert!(store.get_desired_assignment("idle").await?.is_none());
        assert_eq!(
            store.get_desired_assignment("e2").await?.unwrap().run_id,
            outside
        );
        // Nested controllers retain multiple sampler members across root ticks.
        sqlx::query("UPDATE runs SET integration_params=jsonb_set(integration_params, '{run_kind}', to_jsonb('parameter_scan'::text)) WHERE id=$1")
            .bind(left).execute(&harness.pool).await?;
        apply_controller_assignment_plan(&store, ControllerAssignmentPlan::new(root, vec![left]))
            .await?;
        apply_controller_assignment_plan(
            &store,
            ControllerAssignmentPlan::new(left, vec![grandchild]),
        )
        .await?;
        apply_controller_assignment_plan(&store, ControllerAssignmentPlan::new(root, vec![left]))
            .await?;
        let placements = store.list_desired_assignments(None).await?;
        assert_eq!(
            placements
                .iter()
                .filter(|a| a.role == WorkerRole::SamplerAggregator
                    && [left, grandchild].contains(&a.run_id))
                .count(),
            3
        );
        store
            .set_current_assignment("s1", WorkerRole::SamplerAggregator, grandchild)
            .await?;
        assert!(
            store
                .set_current_assignment("s2", WorkerRole::SamplerAggregator, grandchild)
                .await
                .is_err(),
            "execution remains exclusive per child"
        );
        store.clear_current_assignment("s1").await?;
        // Pause-all follows the same semantics through the CLI.
        harness
            .cli()
            .args(["run", "pause", "--all"])
            .assert()
            .success();
        assert_eq!(store.get_registered_workers(Some(root)).await?.len(), 3);
        assert!(store.list_desired_assignments(None).await?.is_empty());
        // Explicit removal also releases paused members, not just placements.
        store.clear_desired_assignments_for_run(root).await?;
        assert!(store.get_registered_workers(Some(root)).await?.is_empty());
        assert_eq!(store.resume_worker_pool(root).await?, 0);
        harness.cleanup().await?;
    }
    Ok(())
}
