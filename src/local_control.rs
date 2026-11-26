//! Durable coordination owned by the combined control/API service.
//! There are deliberately no scheduler concepts here.
use crate::{PgStore, core::RunReadStore, stores::RunLifecycleState};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Connection, Executor, PgConnection};
use std::path::PathBuf;

const ADMISSION_LOCK: i64 = 714831593420020;

pub fn require_local_processes(external: bool, allowed: bool, action: &str) -> Result<()> {
    ensure!(
        !external || allowed,
        "This instance is externally managed. This command would {action} on the current host. Use the deployment helper, or pass --allow-local-processes if you intend to run it here. Avoid starting services or workloads on cluster login nodes unless you know what you are doing."
    );
    Ok(())
}

/// Hold a shared transaction lock throughout a submission. The shutdown request
/// takes its exclusive counterpart, so it cannot miss an in-flight submission.
/// Use an independent connection to avoid starving the server's query pool.
pub async fn admit(store: &PgStore) -> Result<PgConnection> {
    let mut conn = PgConnection::connect_with(&store.pool().connect_options()).await?;
    conn.execute("BEGIN").await?;
    sqlx::query("SELECT pg_advisory_xact_lock_shared($1)")
        .bind(ADMISSION_LOCK)
        .execute(&mut conn)
        .await?;
    let draining: bool =
        sqlx::query_scalar("SELECT draining FROM gammaboard_instance_control WHERE singleton")
            .fetch_one(&mut conn)
            .await
            .context("instance control schema is missing; apply database migrations")?;
    ensure!(
        !draining,
        "instance is draining; new work and launches are blocked"
    );
    Ok(conn)
}

pub async fn attach(store: &PgStore, owner: &str) -> Result<()> {
    sqlx::query("UPDATE gammaboard_instance_control SET owner_id=$1, heartbeat_at=now(),started_at=now(),draining=CASE WHEN shutdown_result IS NULL OR shutdown_result->>'phase'='cleanup_timeout' THEN draining ELSE false END,shutdown_request=CASE WHEN shutdown_result IS NULL OR shutdown_result->>'phase'='cleanup_timeout' THEN shutdown_request ELSE NULL END,shutdown_id=CASE WHEN shutdown_result IS NULL OR shutdown_result->>'phase'='cleanup_timeout' THEN shutdown_id ELSE NULL END,shutdown_result=NULL,cleanup_required=false,cleanup_acknowledged=false WHERE singleton")
        .bind(owner).execute(store.pool()).await?;
    Ok(())
}
pub async fn publish(store: &PgStore, owner: &str, status: &Value) -> Result<()> {
    let changed=sqlx::query("UPDATE gammaboard_instance_control SET heartbeat_at=now(),status=$2 WHERE singleton AND owner_id=$1")
        .bind(owner).bind(status).execute(store.pool()).await?.rows_affected();
    ensure!(changed == 1, "instance supervisor ownership changed");
    Ok(())
}
pub async fn status(store: &PgStore) -> Result<Value> {
    let row:Value=sqlx::query_scalar("SELECT to_jsonb(c) || jsonb_build_object('alive',heartbeat_at>now()-interval '30 seconds') FROM gammaboard_instance_control c WHERE singleton").fetch_one(store.pool()).await?;
    let mut value = row["status"].as_object().cloned().unwrap_or_default();
    value.insert("stop_id".into(), row["shutdown_id"].clone());
    value.insert("stop_result".into(), row["shutdown_result"].clone());
    value.insert("shutdown_request".into(), row["shutdown_request"].clone());
    value.insert("cleanup_required".into(), row["cleanup_required"].clone());
    value.insert(
        "cleanup_acknowledged".into(),
        row["cleanup_acknowledged"].clone(),
    );
    value.insert("supervisor_reachable".into(), row["alive"].clone());
    if row["alive"] != true && value.get("state").is_none_or(|s| s != "stopped") {
        value.insert("state".into(), "unreachable".into());
    }
    if row["shutdown_result"]["phase"] == "cleanup_timeout" && row["alive"] == true {
        value.insert("state".into(), "cleanup_failed".into());
    }
    value.insert(
        "exports".into(),
        serde_json::to_value(exports(store).await?)?,
    );
    let version: Option<i32> =
        sqlx::query_scalar("SELECT model_version FROM gammaboard_compatibility WHERE singleton")
            .fetch_optional(store.pool())
            .await?;
    value.insert(
        "database".into(),
        json!({"reachable":true,"model_version":version}),
    );
    Ok(Value::Object(value))
}
pub async fn mark_started(store: &PgStore, run: i32) -> Result<()> {
    sqlx::query("INSERT INTO gammaboard_run_starts(run_id) VALUES($1) ON CONFLICT(run_id) DO UPDATE SET started_at=now()").bind(run).execute(store.pool()).await?;
    Ok(())
}

pub async fn request_stop(
    store: &PgStore,
    component: Option<&str>,
    wait: bool,
    timeout_ms: Option<u64>,
) -> Result<String> {
    let mut tx = store.pool().begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(ADMISSION_LOCK)
        .execute(&mut *tx)
        .await?;
    let row:Value=sqlx::query_scalar("SELECT to_jsonb(c) || jsonb_build_object('alive',heartbeat_at>now()-interval '30 seconds') FROM gammaboard_instance_control c WHERE singleton FOR UPDATE").fetch_one(&mut *tx).await?;
    ensure!(
        row["alive"] == true,
        "no live instance supervisor is registered; use the deployment helper to inspect or clean up unavailable services"
    );
    if row["draining"] == true
        && ((wait && row["shutdown_result"].is_null())
            || matches!(
                row["shutdown_result"]["phase"].as_str(),
                Some("ready_to_exit" | "closing_database")
            ))
    {
        return Ok(row["shutdown_id"]
            .as_str()
            .context("missing shutdown ID")?
            .into());
    }
    let mut runs = Vec::new();
    if wait {
        runs = store
            .get_all_runs()
            .await?
            .into_iter()
            .filter(|r| {
                r.parent_run_id.is_none()
                    && matches!(
                        r.lifecycle_state,
                        RunLifecycleState::Running | RunLifecycleState::Pausing
                    )
            })
            .map(|r| r.run_id)
            .collect();
        let started:Vec<i32>=sqlx::query_scalar("SELECT s.run_id FROM gammaboard_run_starts s JOIN gammaboard_instance_control c ON c.singleton WHERE s.started_at>=c.started_at").fetch_all(&mut *tx).await?;
        runs.extend(started);
        runs.sort_unstable();
        runs.dedup();
    }
    // Once admission is closed, unclaimed local launches must not reappear on restart.
    // A local controller holds admission while fulfilling a claim, so none can
    // race this transaction and spawn after shutdown takes ownership.
    if component.is_none() {
        sqlx::query("WITH canceled AS (UPDATE node_launch_requests SET state='canceled',error='instance shutdown before launch',updated_at=now() WHERE backend='local' AND state='pending' RETURNING id) UPDATE nodes SET resume_requested=false,shutdown_requested_at=now(),updated_at=now() WHERE launch_request_id IN (SELECT id FROM canceled)").execute(&mut *tx).await?;
    }
    let id = uuid::Uuid::new_v4().to_string();
    let request = json!({"id":id,"component":component,"wait":wait,"runs":runs,"timeout_ms":timeout_ms,"requested_at":chrono::Utc::now()});
    sqlx::query("UPDATE gammaboard_instance_control SET shutdown_id=$1,shutdown_request=$2,shutdown_result=NULL,draining=$3,cleanup_required=false,cleanup_acknowledged=false WHERE singleton")
        .bind(&id).bind(request).bind(component.is_none() || component==Some("db")).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(id)
}
pub async fn finish_stop(store: &PgStore, id: &str, result: &Value) -> Result<()> {
    sqlx::query("UPDATE gammaboard_instance_control SET shutdown_result=$2,draining=COALESCE($2->>'phase' IN ('closing_database','ready_to_exit','cleanup_timeout'),false),cleanup_required=COALESCE($2->>'phase'='cleanup_timeout',false) WHERE singleton AND shutdown_id=$1")
        .bind(id).bind(result).execute(store.pool()).await?;
    Ok(())
}
pub async fn require_external_cleanup(store: &PgStore, id: &str) -> Result<bool> {
    // Submitted receipts are retained even after workers disconnect. The external
    // owner confirms its allocations are gone; GammaBoard never guesses scheduler state.
    let external:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM external_worker_launches WHERE NOT cleanup_confirmed AND (job_id IS NOT NULL OR state IN ('pending','submitting','submitted'))) OR EXISTS(SELECT 1 FROM node_launch_requests WHERE backend='external' AND state IN ('pending','starting'))").fetch_one(store.pool()).await?;
    if !external {
        return Ok(false);
    }
    let acknowledged:Option<bool>=sqlx::query_scalar("UPDATE gammaboard_instance_control SET cleanup_required=true WHERE singleton AND shutdown_id=$1 RETURNING cleanup_acknowledged")
        .bind(id).fetch_optional(store.pool()).await?;
    Ok(!acknowledged.unwrap_or(false))
}
pub async fn acknowledge_cleanup(store: &PgStore, id: &str) -> Result<()> {
    let mut tx = store.pool().begin().await?;
    let count=sqlx::query("UPDATE gammaboard_instance_control SET cleanup_acknowledged=true WHERE singleton AND shutdown_id=$1 AND cleanup_required").bind(id).execute(&mut *tx).await?.rows_affected();
    ensure!(
        count == 1,
        "shutdown request changed or does not require external cleanup"
    );
    sqlx::query("UPDATE external_worker_launches SET cleanup_confirmed=true,state=CASE WHEN state IN ('pending','submitting','submitted') THEN 'canceled' ELSE state END,updated_at=now() WHERE NOT cleanup_confirmed").execute(&mut *tx).await?;
    sqlx::query("UPDATE node_launch_requests SET state='canceled',launcher_token=NULL,launcher_lease_expires_at=NULL,updated_at=now() WHERE backend='external' AND state IN ('pending','starting')").execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Export {
    pub id: String,
    pub run_id: i32,
    pub path: String,
    pub completed: bool,
    pub error: Option<String>,
}
pub async fn register_export(store: &PgStore, run: i32, path: &std::path::Path) -> Result<String> {
    let mut tx = store.pool().begin().await?;
    let path = path.to_string_lossy().into_owned();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,71809242))")
        .bind(&path)
        .execute(&mut *tx)
        .await?;
    if let Some(id)=sqlx::query_scalar::<_,String>("SELECT id FROM gammaboard_run_exports WHERE run_id=$1 AND path=$2 ORDER BY created_at LIMIT 1")
        .bind(run).bind(&path).fetch_optional(&mut *tx).await? {return Ok(id);}
    ensure!(
        !std::path::Path::new(&path).exists(),
        "export output already exists: {path}"
    );
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO gammaboard_run_exports(id,run_id,path) VALUES($1,$2,$3)")
        .bind(&id)
        .bind(run)
        .bind(path)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn import_export(store: &PgStore, export: &Export) -> Result<()> {
    sqlx::query("INSERT INTO gammaboard_run_exports(id,run_id,path,completed,error) SELECT $1,$2,$3,$4,$5 WHERE EXISTS(SELECT 1 FROM runs WHERE id=$2) ON CONFLICT(id) DO NOTHING")
        .bind(&export.id).bind(export.run_id).bind(&export.path).bind(export.completed).bind(&export.error).execute(store.pool()).await?;
    Ok(())
}
pub async fn exports(store: &PgStore) -> Result<Vec<Export>> {
    Ok(sqlx::query_as(
        "SELECT id,run_id,path,completed,error FROM gammaboard_run_exports ORDER BY created_at,id",
    )
    .fetch_all(store.pool())
    .await?)
}
pub async fn claim_export(store: &PgStore, owner: &str) -> Result<Option<Export>> {
    Ok(sqlx::query_as("UPDATE gammaboard_run_exports SET lease_owner=$1,lease_until=now()+interval '60 seconds' WHERE id=(SELECT e.id FROM gammaboard_run_exports e WHERE NOT completed AND (lease_until IS NULL OR lease_until<now()) AND (EXISTS(SELECT 1 FROM run_tasks t WHERE t.run_id=e.run_id AND t.state='failed') OR NOT EXISTS(SELECT 1 FROM run_tasks t WHERE t.run_id=e.run_id AND t.state NOT IN ('completed','failed'))) ORDER BY e.created_at FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING id,run_id,path,completed,error")
        .bind(owner).fetch_optional(store.pool()).await?)
}
pub async fn finish_export(
    store: &PgStore,
    owner: &str,
    id: &str,
    error: Option<String>,
) -> Result<()> {
    sqlx::query("UPDATE gammaboard_run_exports SET completed=$3 IS NULL,error=$3,lease_owner=NULL,lease_until=now()+interval '2 seconds' WHERE id=$1 AND lease_owner=$2")
        .bind(id).bind(owner).bind(error).execute(store.pool()).await?;
    Ok(())
}
pub async fn process_exports(store: &PgStore, owner: &str) -> Result<()> {
    for _ in 0..8 {
        let Some(export) = claim_export(store, owner).await? else {
            break;
        };
        let result: Result<()> = async {
            let mut bundle = crate::api::results::run_results(store, export.run_id, None).await?;
            bundle["export_id"] = export.id.clone().into();
            let path = PathBuf::from(&export.path);
            if path.exists() {
                let existing: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
                ensure!(
                    existing["export_id"] == export.id
                        && existing["run"]["run_id"] == export.run_id,
                    "output belongs to another writer: {}",
                    path.display()
                );
            } else {
                let parent = path.parent().context("export needs a parent path")?;
                std::fs::create_dir_all(parent)?;
                let mut file = tempfile::NamedTempFile::new_in(parent)?;
                serde_json::to_writer_pretty(&mut file, &bundle)?;
                file.as_file().sync_all()?;
                file.persist_noclobber(&path).map_err(|e| e.error)?;
            }
            Ok(())
        }
        .await;
        finish_export(
            store,
            owner,
            &export.id,
            result.err().map(|e| format!("{e:#}")),
        )
        .await?;
    }
    Ok(())
}
