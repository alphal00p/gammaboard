use super::PgStore;
use serde_json::{Value, json};

impl PgStore {
    /// Reserve names and persist their complete launch recipe before either backend runs.
    pub async fn reserve_worker_launch(
        &self,
        backend: &str,
        groups: Vec<Value>,
    ) -> Result<i64, sqlx::Error> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(71809241)")
            .execute(&mut *tx)
            .await?;
        let names: Vec<String> = sqlx::query_scalar("SELECT name FROM nodes")
            .fetch_all(&mut *tx)
            .await?;
        let mut taken = names.into_iter().collect::<std::collections::HashSet<_>>();
        let mut normalized = Vec::new();
        for mut group in groups {
            let count = group["count"].as_u64().unwrap_or(0);
            let prefix = group["name_prefix"].as_str().unwrap_or("w").to_owned();
            let mut names = Vec::new();
            let mut index = 1;
            for _ in 0..count {
                loop {
                    let name = format!("{prefix}-{index}");
                    index += 1;
                    if taken.insert(name.clone()) {
                        names.push(name);
                        break;
                    }
                }
            }
            group["node_names"] = json!(names);
            normalized.push(group);
        }
        let count: i32 = normalized
            .iter()
            .map(|g| g["count"].as_i64().unwrap_or(0) as i32)
            .sum();
        let args = json!({"groups":normalized});
        let id: i64 = sqlx::query_scalar("INSERT INTO node_launch_requests (state,backend,requested_count,args) VALUES ('pending',$1,$2,$3) RETURNING id")
            .bind(backend).bind(count).bind(&args).fetch_one(&mut *tx).await?;
        for group in &normalized {
            for name in group["node_names"].as_array().into_iter().flatten() {
                let mut recipe = group.clone();
                recipe["count"] = json!(1);
                recipe["node_names"] = json!([name]);
                sqlx::query("INSERT INTO nodes (name,uuid,lease_expires_at,launch_group,launch_request_id) VALUES ($1,'',to_timestamp(0),$2,$3)")
                    .bind(name.as_str().unwrap()).bind(recipe).bind(id).execute(&mut *tx).await?;
            }
        }
        tx.commit().await?;
        Ok(id)
    }

    /// Save only live workers, before requesting shutdown. Repeating this is harmless.
    pub async fn suspend_workers(&self) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("UPDATE nodes SET resume_requested=true, shutdown_requested_at=now(), updated_at=now() WHERE lease_expires_at > now() AND shutdown_requested_at IS NULL")
            .execute(self.pool()).await?;
        Ok(result.rows_affected())
    }

    /// Clear the marker in the same transaction that durably enqueues its replacement.
    pub async fn enqueue_resumed_workers(&self) -> Result<usize, sqlx::Error> {
        let mut tx = self.pool().begin().await?;
        let rows: Vec<(String, Value, String)> = sqlx::query_as("SELECT n.name,n.launch_group,r.backend FROM nodes n JOIN node_launch_requests r ON r.id=n.launch_request_id WHERE n.resume_requested AND n.lease_expires_at <= now() AND n.launch_group IS NOT NULL ORDER BY n.name FOR UPDATE OF n")
            .fetch_all(&mut *tx).await?;
        for (name, group, backend) in &rows {
            let id: i64 = sqlx::query_scalar("INSERT INTO node_launch_requests (state,backend,requested_count,args) VALUES ('pending',$1,1,$2) RETURNING id")
                .bind(backend).bind(json!({"groups":[group]})).fetch_one(&mut *tx).await?;
            sqlx::query("UPDATE nodes SET resume_requested=false,launch_request_id=$2,shutdown_requested_at=NULL,updated_at=now() WHERE name=$1")
                .bind(name).bind(id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        let unknown: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM nodes WHERE resume_requested AND launch_group IS NULL",
        )
        .fetch_one(self.pool())
        .await?;
        if unknown > 0 {
            tracing::warn!(
                workers = unknown,
                "cannot resume workers without a recorded launch request; start them through the normal launcher"
            );
        }
        Ok(rows.len())
    }

    pub async fn claim_local_worker_launch(&self) -> Result<Option<(i64, Value)>, sqlx::Error> {
        sqlx::query_as("WITH next AS (SELECT id FROM node_launch_requests WHERE state='pending' AND backend='local' ORDER BY id FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE node_launch_requests r SET state='starting',updated_at=now() FROM next WHERE r.id=next.id RETURNING r.id,r.args")
            .fetch_optional(self.pool()).await
    }
}
