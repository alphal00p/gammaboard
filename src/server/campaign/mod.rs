//! On-demand campaign views. Numerical reduction happens here, never in the browser.
mod diagnostics;
mod overview;
use super::*;
use crate::core::{IntegrationCampaignAllocationSpec, IntegrationCampaignStopCondition};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

type CacheSlot = Arc<Mutex<Option<Cached>>>;

#[derive(Default, Clone)]
pub(super) struct CampaignCache(Arc<Mutex<HashMap<String, CacheSlot>>>);
struct Cached {
    at: Instant,
    value: Value,
}
impl CampaignCache {
    async fn slot(&self, key: String) -> CacheSlot {
        let mut slots = self.0.lock().await;
        if let Some(slot) = slots.get(&key) {
            return slot.clone();
        }
        // At most 64 entries, each at most 1 MiB. In-flight entries remain shared.
        if slots.len() >= 64 {
            if let Some(old) = slots
                .iter()
                .find(|(_, v)| Arc::strong_count(v) == 1)
                .map(|(k, _)| k.clone())
            {
                slots.remove(&old);
            } else {
                return Arc::new(Mutex::new(None));
            }
        }
        let slot = Arc::new(Mutex::new(None));
        slots.insert(key, slot.clone());
        slot
    }
    async fn invalidate(&self) {
        self.0.lock().await.clear();
    }
}

#[derive(Default, Deserialize, Serialize)]
pub(super) struct CampaignQuery {
    #[serde(default = "settings_section")]
    section: String,
    group: Option<String>,
    // URL values need typed deserialization at this level. Serde flatten
    // buffers them as strings, bypassing serde_urlencoded's numeric parsing.
    window_seconds: Option<f64>,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
    node_name: Option<String>,
}
impl CampaignQuery {
    fn range(&self) -> performance_panels::history::RangeQuery {
        performance_panels::history::RangeQuery {
            window_seconds: self.window_seconds,
            start_ms: self.start_ms,
            end_ms: self.end_ms,
            node_name: self.node_name.clone(),
        }
    }
}
fn settings_section() -> String {
    "settings".into()
}

pub(super) async fn get(
    State(state): State<AppState>,
    AxumPath((run, task)): AxumPath<(i32, i64)>,
    Query(query): Query<CampaignQuery>,
) -> Result<Json<Value>, ApiError> {
    if !["settings", "overview", "diagnostics"].contains(&query.section.as_str()) {
        return Err(ApiError::BadRequest("Unknown campaign section".into()));
    }
    let range = query.range();
    range.validate()?;
    let task = load_run_task(&state.store, run, task).await?;
    if !matches!(task.task, RunTaskSpec::IntegrationCampaign { .. }) {
        return Err(ApiError::BadRequest(
            "Task is not an integration campaign".into(),
        ));
    }
    if query.section == "settings" {
        return json_response(metadata(&state.store, run, &task).await?);
    }
    let key = format!(
        "{run}:{}:{}",
        task.id,
        serde_json::to_string(&query).unwrap()
    );
    let slot = state.campaign_cache.slot(key).await;
    let mut cached = slot.lock().await;
    if let Some(saved) = cached
        .as_ref()
        .filter(|v| v.at.elapsed() < Duration::from_secs(8))
    {
        return json_response(saved.value.clone());
    }
    let mut result = metadata(&state.store, run, &task).await?;
    let response = if query.section == "diagnostics" {
        diagnostics::load(&state.store, run, &task, query.group.as_deref()).await?
    } else {
        overview::load(&state.store, run, task.id, &range).await?
    };
    result
        .as_object_mut()
        .unwrap()
        .extend(response.as_object().unwrap().clone());
    if serde_json::to_vec(&result).is_ok_and(|v| v.len() <= 1024 * 1024) {
        *cached = Some(Cached {
            at: Instant::now(),
            value: result.clone(),
        });
    }
    json_response(result)
}

async fn metadata(store: &PgStore, run: i32, task: &RunTask) -> Result<Value, ApiError> {
    let rows: Vec<Value> = sqlx::query_scalar(r#"
        SELECT jsonb_build_object('run_id',r.id,'run_name',r.name,'label',r.spawn_label,
            'defaults',r.integration_params#>'{sampler_aggregator_runner_params,queue}',
            'tasks',COALESCE((SELECT jsonb_agg(jsonb_build_object('id',t.id::text,'name',t.name,'state',t.state,
                'publish_result',COALESCE((t.task->>'publish_result')::bool,true),'queue_tuning',t.task->'queue_tuning') ORDER BY t.sequence_nr)
                FROM run_tasks t WHERE t.run_id=r.id AND t.task->>'kind'='sample'),'[]'))
        FROM runs r WHERE r.parent_run_id=$1 AND r.parent_task_id=$2 AND r.spawn_kind='integration_campaign' ORDER BY r.id
    "#).bind(run).bind(task.id).fetch_all(store.pool()).await.map_err(crate::core::StoreError::from)?;
    let RunTaskSpec::IntegrationCampaign {
        children,
        allocation,
        ..
    } = &task.task
    else {
        unreachable!()
    };
    let children = children
        .iter()
        .map(|child| {
            let mut row = rows
                .iter()
                .find(|r| r["label"] == child.name)
                .cloned()
                .unwrap_or(json!({"label":child.name,"run_id":null,"tasks":[]}));
            row["coefficient"] = json!(child.coefficient);
            row
        })
        .collect::<Vec<_>>();
    let counts: (i64,i64)=sqlx::query_as(r#"SELECT count(*) FILTER(WHERE desired_role='sampler_aggregator'),count(*) FILTER(WHERE desired_role='evaluator')
        FROM nodes WHERE lease_expires_at>now() AND (desired_run_id=$1 OR desired_run_id IN (SELECT id FROM runs WHERE parent_run_id=$1 AND parent_task_id=$2))"#)
        .bind(run).bind(task.id).fetch_one(store.pool()).await.map_err(crate::core::StoreError::from)?;
    let reserved: i64=sqlx::query_scalar("SELECT COALESCE(sum(granted_samples),0)::bigint FROM campaign_sample_budgets WHERE campaign_task_id=$1")
        .bind(task.id).fetch_one(store.pool()).await.map_err(crate::core::StoreError::from)?;
    let rules: Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('child',child_label,'scope',task_scope,'patch',patch) FROM campaign_queue_tuning_rules WHERE campaign_task_id=$1 ORDER BY updated_at,id")
        .bind(task.id).fetch_all(store.pool()).await.map_err(crate::core::StoreError::from)?;
    Ok(
        json!({"run_id":run,"task_id":task.id.to_string(),"state":task.state,"settings":crate::stores::pg_store::campaign::settings(&task.task),
        "children":children,"rules":rules,"reserved_samples":reserved,"samplers":counts.0,"evaluators":counts.1,
        "capacity":allocation.max_active_runs.min(counts.0 as usize).min(counts.1 as usize/allocation.min_evaluators_per_run),
        "controller_output":task.controller_output,"poll_after_ms":10000}),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SchedulingEdit {
    expected: Value,
    allocation: IntegrationCampaignAllocationSpec,
    stop_condition: IntegrationCampaignStopCondition,
}
pub(super) async fn scheduling(
    State(state): State<AppState>,
    AxumPath((run, task)): AxumPath<(i32, i64)>,
    AxumJson(edit): AxumJson<SchedulingEdit>,
) -> Result<Json<Value>, ApiError> {
    state
        .store
        .update_campaign_scheduling(
            run,
            task,
            edit.expected,
            edit.allocation,
            edit.stop_condition,
        )
        .await?;
    state.campaign_cache.invalidate().await;
    tracing::info!(
        source = "control",
        action = "campaign_scheduling",
        run_id = run,
        task_id = task,
        "campaign scheduling updated"
    );
    json_response(json!({"updated":true}))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TuningEdit {
    children: Vec<String>,
    scope: String,
    patch: Value,
}
pub(super) async fn tuning(
    State(state): State<AppState>,
    AxumPath((run, task)): AxumPath<(i32, i64)>,
    AxumJson(edit): AxumJson<TuningEdit>,
) -> Result<Json<Value>, ApiError> {
    let result = state
        .store
        .update_campaign_queue_tuning(run, task, &edit.children, &edit.scope, edit.patch)
        .await?;
    state.campaign_cache.invalidate().await;
    tracing::info!(
        source = "control",
        action = "campaign_queue_tuning",
        run_id = run,
        task_id = task,
        "campaign queue tuning updated"
    );
    json_response(result)
}

fn response(source: String, panels: Vec<(&str, panels::PanelState)>) -> Value {
    let mut specs = vec![];
    let mut updates = vec![];
    for (label, panel) in panels {
        let value = serde_json::to_value(panel).unwrap();
        specs.push(json!({"panel_id":value["panel_id"],"label":label,"kind":value["kind"],"history":"none","width":"full"}));
        updates.push(json!({"mode":"replace","panel":value}));
    }
    json!({"source_id":source,"panels":specs,"updates":updates,"poll_after_ms":10000})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn campaign_query_parses_numeric_time_windows_from_urls() {
        for (encoded, seconds) in [("30", 30.0), ("300", 300.0), ("30.0", 30.0), ("0.25", 0.25)] {
            let uri = format!("/campaign?section=overview&window_seconds={encoded}")
                .parse()
                .unwrap();
            let Query(query) = Query::<CampaignQuery>::try_from_uri(&uri).unwrap();
            assert_eq!(query.range().window_seconds, Some(seconds));
            query.range().validate().unwrap();
        }
        let uri = "/campaign?section=overview&start_ms=1000&end_ms=2000"
            .parse()
            .unwrap();
        let Query(query) = Query::<CampaignQuery>::try_from_uri(&uri).unwrap();
        assert_eq!(
            (query.range().start_ms, query.range().end_ms),
            (Some(1000), Some(2000))
        );
        query.range().validate().unwrap();

        let uri = "/campaign?section=overview".parse().unwrap();
        let Query(query) = Query::<CampaignQuery>::try_from_uri(&uri).unwrap();
        assert_eq!(query.range().window_seconds, None);
        query.range().validate().unwrap();
    }

    #[tokio::test]
    #[ignore = "requires an isolated migrated postgres database"]
    async fn campaign_views_read_existing_live_and_saved_children_without_new_snapshots() {
        use crate::core::{ControlPlaneStore, RunTaskInput, RunTaskStore};
        use crate::evaluation::{GammaLoopAccumulatorState, Point};
        let pool = sqlx::PgPool::connect(&std::env::var("GAMMABOARD_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
        let store = PgStore::new(pool);
        let parent:i32=sqlx::query_scalar("INSERT INTO runs(name,point_spec,integration_params) VALUES('campaign-view-test','{}','{\"run_kind\":\"integration_campaign\"}') RETURNING id").fetch_one(store.pool()).await.unwrap();
        let spec:RunTaskSpec=serde_json::from_value(json!({"kind":"integration_campaign","children":[{"name":"A","coefficient":2,"run":"name='a'"},{"name":"B","coefficient":-1,"run":"name='b'"}],"stop_condition":{"max_total_samples":1000}})).unwrap();
        let task = store
            .append_run_tasks(
                parent,
                &[RunTaskInput {
                    name: Some("campaign".into()),
                    task: spec,
                }],
            )
            .await
            .unwrap()
            .remove(0);
        sqlx::query("INSERT INTO nodes(name,uuid,lease_expires_at) VALUES('reused-worker','node',now()-interval '1 hour')").execute(store.pool()).await.unwrap();
        for (label, values, active) in [
            ("A", vec![2., 4.], true),
            ("B", vec![1., 1., 1., 5.], false),
        ] {
            let run:i32=sqlx::query_scalar("INSERT INTO runs(name,point_spec,integration_params,parent_run_id,parent_task_id,spawn_kind,spawn_label) VALUES($1,'{}','{}',$2,$3,'integration_campaign',$1) RETURNING id").bind(label).bind(parent).bind(task.id).fetch_one(store.pool()).await.unwrap();
            let sample:RunTaskSpec=serde_json::from_value(json!({"kind":"sample","publish_result":true,"stop_condition":{"max_samples":100},"evaluator":{"config":{"kind":"unit"}},"sampler":{"config":{"kind":"naive_monte_carlo"}}})).unwrap();
            let child_task = store
                .append_run_tasks(
                    run,
                    &[RunTaskInput {
                        name: Some("infer".into()),
                        task: sample,
                    }],
                )
                .await
                .unwrap()
                .remove(0);
            let mut acc = GammaLoopAccumulatorState::default();
            for value in &values {
                acc.estimate
                    .ingest_vector(&[*value, 0.0], &Point::new(vec![0.5], vec![], 1.0))
                    .unwrap();
            }
            acc.diagnostics.count_total = values.len() as i64;
            acc.diagnostics.total_eval_time_ms = values.len() as f64 * 10.;
            acc.diagnostics.total_integrand_eval_time_ms = values.len() as f64 * 8.;
            let mut raw = serde_json::to_value(AccumulatorState::Gammaloop(acc)).unwrap();
            // Invalid histogram payload proves this diagnostic request excludes it before decoding.
            raw["bundle"]["histograms"] =
                json!({"large-unused-histogram":{"intentionally":"not a histogram"}});
            sqlx::query("UPDATE run_tasks SET state=$2,nr_produced_samples=$3,nr_completed_samples=$3 WHERE id=$1").bind(child_task.id).bind(if active {"active"}else{"completed"}).bind(values.len() as i64).execute(store.pool()).await.unwrap();
            sqlx::query(
                "UPDATE runs SET nr_produced_samples=$2,nr_completed_samples=$2 WHERE id=$1",
            )
            .bind(run)
            .bind(values.len() as i64)
            .execute(store.pool())
            .await
            .unwrap();
            if active {
                sqlx::query("UPDATE runs SET current_observable=$2 WHERE id=$1")
                    .bind(run)
                    .bind(&raw)
                    .execute(store.pool())
                    .await
                    .unwrap();
            } else {
                sqlx::query("INSERT INTO run_stage_snapshots(run_id,task_id,name,sequence_nr,queue_empty,observable_state,batch_transforms) VALUES($1,$2,'infer',0,true,$3,'[]')").bind(run).bind(child_task.id).bind(&raw).execute(store.pool()).await.unwrap();
            }
            sqlx::query(
                "INSERT INTO run_telemetry_workers(run_id,worker_id) VALUES($1,'reused-worker')",
            )
            .bind(run)
            .execute(store.pool())
            .await
            .unwrap();
            for (time, n) in [(100, 0), (110, if active { 20 } else { 40 })] {
                let metrics = json!({"epoch":label,"node_uuid":"node","task_id":child_task.id.to_string(),"completed_samples_total":n,"busy":{"elapsed_seconds":time-100,"compute_seconds":(time-100) as f64*0.2,"io_seconds":0}});
                sqlx::query("INSERT INTO sampler_aggregator_performance_history(run_id,worker_id,metrics,created_at) VALUES($1,'reused-worker',$2,to_timestamp($3))").bind(run).bind(metrics).bind(time as f64).execute(store.pool()).await.unwrap();
            }
        }
        let data = diagnostics::load(&store, parent, &task, None)
            .await
            .unwrap();
        assert_eq!(data["groups"].as_array().unwrap().len(), 1);
        let panels = data["updates"].as_array().unwrap();
        let points = &panels
            .iter()
            .find(|p| p["panel"]["panel_id"] == "max_weight_points")
            .unwrap()["panel"];
        assert_eq!(points["rows"][0][7], 4.0);
        let evaluation = &panels
            .iter()
            .find(|p| p["panel"]["panel_id"] == "gammaloop_evaluation_diagnostics")
            .unwrap()["panel"];
        assert_eq!(
            evaluation["entries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["key"] == "count_total")
                .unwrap()["value"],
            6
        );
        // Older tasks may omit default fields and retain only a bare persistent
        // observable snapshot. The view must still work without a controller rerun.
        sqlx::query("UPDATE run_tasks SET task=task-'publish_result' WHERE state='completed' AND run_id IN (SELECT id FROM runs WHERE parent_run_id=$1)").bind(parent).execute(store.pool()).await.unwrap();
        sqlx::query("INSERT INTO persisted_observable_snapshots(run_id,task_id,persisted_observable) SELECT s.run_id,s.task_id,s.observable_state-'kind' FROM run_stage_snapshots s JOIN runs r ON r.id=s.run_id WHERE r.parent_run_id=$1").bind(parent).execute(store.pool()).await.unwrap();
        sqlx::query("DELETE FROM run_stage_snapshots WHERE run_id IN (SELECT id FROM runs WHERE parent_run_id=$1)").bind(parent).execute(store.pool()).await.unwrap();
        let legacy = diagnostics::load(&store, parent, &task, None)
            .await
            .unwrap();
        assert_eq!(legacy["updates"], data["updates"]);
        let snapshots:i64=sqlx::query_scalar("SELECT count(*) FROM persisted_observable_snapshots WHERE run_id IN (SELECT id FROM runs WHERE parent_run_id=$1)").bind(parent).fetch_one(store.pool()).await.unwrap();
        assert_eq!(snapshots, 1, "dashboard reads do not create snapshots");
        let data = overview::load(&store, parent, task.id, &Default::default())
            .await
            .unwrap();
        let overview = &data["updates"][0]["panel"]["entries"];
        assert_eq!(
            overview
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["key"] == "accepted_rate")
                .unwrap()["value"],
            6.0
        );
        assert_eq!(
            overview
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["key"] == "samples")
                .unwrap()["value"],
            6
        );
        let meta = metadata(&store, parent, &task).await.unwrap();
        assert_eq!(meta["children"].as_array().unwrap().len(), 2);
        assert_eq!(meta["children"][0]["tasks"][0]["name"], "infer");
        store.remove_run(parent).await.unwrap();
    }
}
