use super::PgStore;
use serde_json::{Value, json};

#[derive(sqlx::FromRow)]
struct OfflineLaunchRecipe {
    launch_group: Option<Value>,
    backend: Option<String>,
    args: Option<Value>,
    pending: bool,
    restore_requested: bool,
}

impl PgStore {
    pub async fn offline_nodes(&self) -> Result<Vec<Value>, sqlx::Error> {
        sqlx::query_scalar(r#"
            SELECT jsonb_build_object(
                'node_name',n.name,'node_uuid',n.uuid,'last_seen',n.last_seen,
                'pool_run_id',n.pool_run_id,'pool_run_name',r.name,'pool_role',n.pool_role,
                'restore_requested',n.resume_requested,'backend',l.backend,
                'launch_state',l.state,'launch_pending',p.pending,
                'can_start',n.launch_group IS NOT NULL AND l.id IS NOT NULL AND NOT p.pending,
                'status',CASE WHEN p.pending THEN 'starting' ELSE 'offline' END)
            FROM nodes n LEFT JOIN runs r ON r.id=n.pool_run_id
            LEFT JOIN node_launch_requests l ON l.id=n.launch_request_id
            CROSS JOIN LATERAL (SELECT worker_launch_pending(n.name,n.launch_request_id,n.last_seen) AS pending) p
            WHERE n.lease_expires_at<=now() ORDER BY n.name
        "#).fetch_all(self.pool()).await
    }

    /// Queue one saved worker recipe without changing its name or assignments.
    pub async fn start_offline_node(
        &self,
        name: &str,
        uuid: &str,
        allow_local: bool,
        restore_only: bool,
    ) -> Result<i64, sqlx::Error> {
        let mut tx = self.pool().begin().await?;
        super::connection_budget::lock(&mut tx).await?;
        let row: Option<OfflineLaunchRecipe> = sqlx::query_as(
            "SELECT n.launch_group,l.backend,l.args,n.resume_requested AS restore_requested,worker_launch_pending(n.name,n.launch_request_id,n.last_seen) AS pending FROM nodes n LEFT JOIN node_launch_requests l ON l.id=n.launch_request_id WHERE n.name=$1 AND n.uuid=$2 AND n.lease_expires_at<=now() FOR UPDATE OF n"
        ).bind(name).bind(uuid).fetch_optional(&mut *tx).await?;
        let Some(OfflineLaunchRecipe {
            launch_group: Some(group),
            backend: Some(backend),
            args: Some(mut args),
            pending,
            restore_requested,
        }) = row
        else {
            return Err(sqlx::Error::Protocol("Node is live, changed identity, or has no saved launch recipe. Refresh the node list.".into()));
        };
        if pending {
            return Err(sqlx::Error::Protocol(
                "This worker already has a pending launch.".into(),
            ));
        }
        if restore_only && !restore_requested {
            return Err(sqlx::Error::Protocol(
                "This worker is no longer saved for restart. Refresh the worker list.".into(),
            ));
        }
        if backend == "local" && !allow_local {
            return Err(sqlx::Error::Protocol(
                "Local worker launch is disabled on this server.".into(),
            ));
        }
        super::connection_budget::check_worker_connection_budget(&mut tx, 1).await?;
        args["groups"] = json!([group]);
        let id: i64 = sqlx::query_scalar("INSERT INTO node_launch_requests (state,backend,requested_count,args) VALUES ('pending',$1,1,$2) RETURNING id")
            .bind(backend).bind(args).fetch_one(&mut *tx).await?;
        sqlx::query("UPDATE nodes SET resume_requested=false,launch_request_id=$2,shutdown_requested_at=NULL,updated_at=now() WHERE name=$1")
            .bind(name).bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(id)
    }

    pub async fn unassign_offline_node(&self, name: &str, uuid: &str) -> Result<bool, sqlx::Error> {
        self.update_offline_worker(
            name,
            uuid,
            "pool_run_id=NULL,pool_role=NULL,desired_run_id=NULL,desired_role=NULL,active_run_id=NULL,active_role=NULL",
        ).await
    }

    pub async fn cancel_worker_restoration(
        &self,
        name: &str,
        uuid: &str,
    ) -> Result<bool, sqlx::Error> {
        self.update_offline_worker(name, uuid, "resume_requested=false")
            .await
    }

    // Only fixed internal SET clauses reach this helper. Both actions fence a
    // stale UI row against a live replacement or an already queued launch.
    async fn update_offline_worker(
        &self,
        name: &str,
        uuid: &str,
        set_clause: &str,
    ) -> Result<bool, sqlx::Error> {
        let mut tx = self.pool().begin().await?;
        super::connection_budget::lock(&mut tx).await?;
        let changed = sqlx::query(&format!(
            r#"
            UPDATE nodes SET {set_clause},updated_at=now()
            WHERE name=$1 AND uuid=$2 AND lease_expires_at<=now()
              AND NOT worker_launch_pending(name,launch_request_id,last_seen)
        "#
        ))
        .bind(name)
        .bind(uuid)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(changed > 0)
    }

    /// Reserve names and persist their complete launch recipe before either backend runs.
    pub async fn reserve_worker_launch(
        &self,
        backend: &str,
        groups: Vec<Value>,
    ) -> Result<i64, sqlx::Error> {
        self.reserve_worker_launch_with_args(backend, groups, json!({}))
            .await
    }

    pub async fn reserve_worker_launch_with_args(
        &self,
        backend: &str,
        groups: Vec<Value>,
        mut args: Value,
    ) -> Result<i64, sqlx::Error> {
        let mut tx = self.pool().begin().await?;
        super::connection_budget::lock(&mut tx).await?;
        let requested = groups.iter().try_fold(0_i64, |total, group| {
            let count = group["count"]
                .as_i64()
                .filter(|count| *count > 0)
                .ok_or_else(|| sqlx::Error::Protocol("worker count must be positive".into()))?;
            total
                .checked_add(count)
                .ok_or_else(|| sqlx::Error::Protocol("worker count overflow".into()))
        })?;
        super::connection_budget::check_worker_connection_budget(&mut tx, requested).await?;
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
        if !args.is_object() {
            args = json!({});
        }
        args["groups"] = json!(normalized);
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
    pub async fn save_workers_for_restart(&self) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("UPDATE nodes SET resume_requested=true, shutdown_requested_at=now(), updated_at=now() WHERE lease_expires_at > now() AND shutdown_requested_at IS NULL")
            .execute(self.pool()).await?;
        Ok(result.rows_affected())
    }

    /// Clear the marker in the same transaction that durably enqueues its replacement.
    pub async fn restore_saved_workers(&self) -> Result<usize, sqlx::Error> {
        let mut tx = self.pool().begin().await?;
        super::connection_budget::lock(&mut tx).await?;
        let rows: Vec<(String, Value, String, Value)> = sqlx::query_as("SELECT n.name,n.launch_group,r.backend,r.args FROM nodes n JOIN node_launch_requests r ON r.id=n.launch_request_id WHERE n.resume_requested AND n.lease_expires_at <= now() AND n.launch_group IS NOT NULL AND NOT worker_launch_pending(n.name,n.launch_request_id,n.last_seen) ORDER BY n.name FOR UPDATE OF n")
            .fetch_all(&mut *tx).await?;
        if !rows.is_empty() {
            super::connection_budget::check_worker_connection_budget(&mut tx, rows.len() as i64)
                .await?;
        }
        for (name, group, backend, original_args) in &rows {
            let mut args = original_args.clone();
            args["groups"] = json!([group]);
            let id: i64 = sqlx::query_scalar("INSERT INTO node_launch_requests (state,backend,requested_count,args) VALUES ('pending',$1,1,$2) RETURNING id")
                .bind(backend).bind(args).fetch_one(&mut *tx).await?;
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
                "cannot restore workers without a recorded launch request; start them through the normal launcher"
            );
        }
        Ok(rows.len())
    }

    pub async fn claim_local_worker_launch(&self) -> Result<Option<(i64, Value)>, sqlx::Error> {
        sqlx::query_as("WITH next AS (SELECT id FROM node_launch_requests WHERE state='pending' AND backend='local' ORDER BY id FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE node_launch_requests r SET state='starting',updated_at=now() FROM next WHERE r.id=next.id RETURNING r.id,r.args")
            .fetch_optional(self.pool()).await
    }
}
