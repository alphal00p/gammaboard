use super::PgStore;
use serde_json::{Value, json};

const LEASE_SECONDS: i32 = 120;

impl PgStore {
    pub async fn bind_deployment(&self, deployment_id: Option<&str>) -> Result<(), sqlx::Error> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("SET LOCAL synchronous_commit=on")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT pg_advisory_xact_lock(71809242)")
            .execute(&mut *tx)
            .await?;
        let bound: Option<(String, String)> = sqlx::query_as(
            "SELECT deployment_id,build_id FROM gammaboard_deployment WHERE singleton",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let build = crate::provenance::build_identity();
        match (deployment_id, bound) {
            (None, None) => {}
            (None, Some(_)) => {
                return Err(sqlx::Error::Protocol(
                    "this database requires GAMMABOARD_DEPLOYMENT_ID".into(),
                ));
            }
            (Some(id), bound) => {
                let replacing = bound
                    .as_ref()
                    .is_none_or(|(old_id, old_build)| old_id != id || old_build != &build);
                if replacing {
                    let live: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM nodes WHERE lease_expires_at>clock_timestamp())")
                        .fetch_one(&mut *tx).await?;
                    if live {
                        return Err(sqlx::Error::Protocol(
                            "cannot replace deployment while old workers are live".into(),
                        ));
                    }
                }
                if replacing && bound.is_some() {
                    sqlx::query("UPDATE node_launch_requests SET state='canceled',error='previous deployment ended before launch completed',updated_at=now() WHERE backend='external' AND state IN ('pending','starting')")
                        .execute(&mut *tx).await?;
                }
                sqlx::query("INSERT INTO gammaboard_deployment(singleton,deployment_id,build_id) VALUES(true,$1,$2) ON CONFLICT(singleton) DO UPDATE SET deployment_id=$1,build_id=$2")
                    .bind(id).bind(build).execute(&mut *tx).await?;
            }
        }
        tx.commit().await
    }

    pub async fn claim_external_launch(
        &self,
        owner: &str,
        request_id: Option<i64>,
    ) -> Result<Option<Value>, sqlx::Error> {
        if owner.trim().is_empty() {
            return Err(sqlx::Error::Protocol(
                "launcher_id must not be empty".into(),
            ));
        }
        let mut tx = self.pool().begin().await?;
        sqlx::query("SET LOCAL synchronous_commit=on")
            .execute(&mut *tx)
            .await?;
        let row: Option<(i64,Value)> = sqlx::query_as(
            "SELECT id,args FROM node_launch_requests WHERE backend='external' AND state IN ('pending','starting') AND ($1::bigint IS NULL OR id=$1)
             AND (launcher_lease_expires_at IS NULL OR launcher_lease_expires_at<clock_timestamp())
             ORDER BY updated_at,id FOR UPDATE SKIP LOCKED LIMIT 1")
            .bind(request_id).fetch_optional(&mut *tx).await?;
        let Some((id, args)) = row else {
            return Ok(None);
        };
        let groups = args["groups"].as_array().ok_or_else(|| {
            sqlx::Error::Protocol("external launch requires grouped arguments".into())
        })?;
        for group in groups {
            for name in group["node_names"].as_array().into_iter().flatten() {
                let name = name
                    .as_str()
                    .ok_or_else(|| sqlx::Error::Protocol("invalid reserved worker name".into()))?;
                sqlx::query("INSERT INTO external_worker_launches(request_id,node_name,submission_key) VALUES($1,$2,$3) ON CONFLICT(request_id,node_name) DO NOTHING")
                    .bind(id).bind(name).bind(uuid::Uuid::new_v4().to_string()).execute(&mut *tx).await?;
            }
        }
        let token = uuid::Uuid::new_v4().to_string();
        sqlx::query("UPDATE node_launch_requests SET state='starting',launcher_token=$2,launcher_owner=$3,launcher_lease_expires_at=clock_timestamp()+make_interval(secs=>$4),updated_at=clock_timestamp() WHERE id=$1")
            .bind(id).bind(&token).bind(owner).bind(LEASE_SECONDS as f64).execute(&mut *tx).await?;
        let workers: Vec<Value> = sqlx::query_scalar("SELECT to_jsonb(w) || jsonb_build_object('registered',EXISTS(SELECT 1 FROM nodes n JOIN node_launch_requests r ON r.id=w.request_id WHERE n.name=w.node_name AND n.launch_request_id=w.request_id AND n.last_seen>=r.created_at)) FROM external_worker_launches w WHERE request_id=$1 ORDER BY node_name")
            .bind(id).fetch_all(&mut *tx).await?;
        tx.commit().await?;
        Ok(Some(
            json!({"id":id.to_string(),"args":args,"state":"starting","lease_token":token,"lease_seconds":LEASE_SECONDS,"workers":workers}),
        ))
    }

    pub async fn heartbeat_external_launch(
        &self,
        id: i64,
        token: &str,
        release: bool,
    ) -> Result<(), sqlx::Error> {
        let changed=sqlx::query("UPDATE node_launch_requests SET launcher_lease_expires_at=CASE WHEN $3 THEN clock_timestamp() ELSE clock_timestamp()+interval '120 seconds' END WHERE id=$1 AND launcher_token=$2 AND launcher_lease_expires_at>clock_timestamp() AND state='starting'")
            .bind(id).bind(token).bind(release).execute(self.pool()).await?;
        if changed.rows_affected() != 1 {
            return Err(sqlx::Error::Protocol(
                "external launch lease expired or was superseded".into(),
            ));
        }
        Ok(())
    }

    pub async fn update_external_worker(
        &self,
        id: i64,
        token: &str,
        name: &str,
        state: &str,
        job_id: Option<&str>,
        error: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("SET LOCAL synchronous_commit=on")
            .execute(&mut *tx)
            .await?;
        let owns: Option<i64>=sqlx::query_scalar("SELECT id FROM node_launch_requests WHERE id=$1 AND backend='external' AND launcher_token=$2 AND launcher_lease_expires_at>clock_timestamp() AND state IN ('starting','fulfilled','failed') FOR UPDATE")
            .bind(id).bind(token).fetch_optional(&mut *tx).await?;
        if owns.is_none() {
            return Err(sqlx::Error::Protocol(
                "external launch lease expired or was superseded".into(),
            ));
        }
        let (old, recorded_job): (String,Option<String>)=sqlx::query_as("SELECT state,job_id FROM external_worker_launches WHERE request_id=$1 AND node_name=$2 FOR UPDATE")
            .bind(id).bind(name).fetch_one(&mut *tx).await?;
        if let Some(job) = job_id
            && (job.is_empty()
                || !job.bytes().all(|b| b.is_ascii_digit())
                || recorded_job.as_deref().is_some_and(|old| old != job)
                || matches!(state, "pending" | "submitting"))
        {
            return Err(sqlx::Error::Protocol(
                "invalid or changed scheduler job identity".into(),
            ));
        }
        // A fast worker may register before sbatch returns its job id. Preserve
        // that authoritative success while accepting its late submission receipt.
        let state = if old == "connected" && matches!(state, "submitted" | "failed" | "canceled") {
            "connected"
        } else {
            state
        };
        let allowed = old == state
            || matches!(
                (old.as_str(), state),
                (
                    "pending",
                    "submitting" | "connected" | "canceled" | "failed"
                ) | (
                    "submitting",
                    "submitted" | "connected" | "failed" | "canceled"
                ) | ("submitted", "connected" | "failed" | "canceled")
            );
        if !allowed
            && !(old == "submitting"
                && state == "pending"
                && job_id.is_none()
                && error == Some("operator confirmed no submission"))
        {
            return Err(sqlx::Error::Protocol(format!(
                "invalid external worker transition {old} -> {state}"
            )));
        }
        if state == "connected" {
            let registered: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM nodes n JOIN node_launch_requests r ON r.id=n.launch_request_id WHERE n.name=$2 AND r.id=$1 AND n.last_seen>=r.created_at)")
                .bind(id).bind(name).fetch_one(&mut *tx).await?;
            if !registered {
                return Err(sqlx::Error::Protocol(
                    "worker has not registered for this request".into(),
                ));
            }
        }
        let error = if state == "connected" { None } else { error };
        if state == "submitted" && job_id.is_none() {
            return Err(sqlx::Error::Protocol(
                "submitted worker requires a scheduler job id".into(),
            ));
        }
        sqlx::query("UPDATE external_worker_launches SET state=$3,job_id=COALESCE($4,job_id),error=$5,updated_at=clock_timestamp() WHERE request_id=$1 AND node_name=$2")
            .bind(id).bind(name).bind(state).bind(job_id).bind(error).execute(&mut *tx).await?;
        // One per-worker update is durable before the next non-transactional sbatch.
        sqlx::query("UPDATE node_launch_requests SET started_count=(SELECT count(*) FROM external_worker_launches WHERE request_id=$1 AND (job_id IS NOT NULL OR state='connected')), result=jsonb_build_object('workers',(SELECT jsonb_agg(to_jsonb(w) ORDER BY node_name) FROM external_worker_launches w WHERE request_id=$1)),updated_at=clock_timestamp() WHERE id=$1")
            .bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE node_launch_requests SET state=CASE WHEN EXISTS(SELECT 1 FROM external_worker_launches WHERE request_id=$1 AND state IN ('failed','canceled')) THEN 'failed' ELSE 'fulfilled' END,error=(SELECT string_agg(error,'; ') FROM external_worker_launches WHERE request_id=$1 AND error IS NOT NULL) WHERE id=$1 AND NOT EXISTS(SELECT 1 FROM external_worker_launches WHERE request_id=$1 AND state IN ('pending','submitting','submitted'))")
            .bind(id).execute(&mut *tx).await?;
        tx.commit().await
    }
}
