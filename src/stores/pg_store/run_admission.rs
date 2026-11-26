//! Atomic admission of a complete worker group and retry-safe creation identities.
use super::*;
use crate::core::{RunReadStore, RunSpecStore};
use serde_json::{Value, json};

const RECORD_START: &str = "INSERT INTO gammaboard_run_starts(run_id) VALUES($1) ON CONFLICT(run_id) DO UPDATE SET started_at=now()";

impl PgStore {
    pub fn with_creation_request(&self, key: String, payload: Value) -> Self {
        let mut scoped = self.clone();
        scoped.creation_request = Some((key, payload));
        scoped
    }
    pub async fn creation_receipt(
        &self,
        key: &str,
        payload: &Value,
    ) -> anyhow::Result<Option<Value>> {
        let row: Option<(Value, Option<i32>)> =
            sqlx::query_as("SELECT payload,run_id FROM run_creation_requests WHERE request_id=$1")
                .bind(key)
                .fetch_optional(&self.pool)
                .await?;
        let Some((expected, id)) = row else {
            return Ok(None);
        };
        anyhow::ensure!(
            &expected == payload,
            "request_id was already used for different creation input"
        );
        let id = id.ok_or_else(|| {
            anyhow::anyhow!(
                "the run for this request_id was deleted; use a new request_id for a new run"
            )
        })?;
        let run = self
            .get_run_progress(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("created run no longer exists"))?;
        Ok(Some(
            json!({"run_id":id,"run_name":run.run_name,"tasks_created":self.list_run_tasks(id).await?.len()}),
        ))
    }

    pub async fn try_start_run(&self, id: i32, cap: Option<usize>) -> anyhow::Result<Value> {
        anyhow::ensure!(cap != Some(0), "max-evaluators must be positive");
        let run = crate::api::nodes::worker_pool_run(self, id).await?;
        anyhow::ensure!(!run.archived, "archived result runs are read-only");
        let id = run.run_id;
        let spec = self
            .load_run_spec(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("run spec missing"))?;
        let tasks = self.list_run_tasks(id).await?;
        let mut tx = self.pool.begin().await?;
        super::super::connection_budget::lock(&mut tx).await?;
        let archived: bool = sqlx::query_scalar("SELECT archived FROM runs WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        anyhow::ensure!(!archived, "archived result runs are read-only");
        if !tasks.is_empty()
            && tasks
                .iter()
                .all(|t| t.state == crate::RunTaskState::Completed)
        {
            return Ok(json!({"run_id":id,"start":{"state":"started","already_completed":true}}));
        }
        let nodes:Vec<Value>=sqlx::query_scalar(r#"SELECT jsonb_build_object(
            'name',name,'uuid',uuid,'capabilities',capabilities,'last_seen',last_seen,
            'pool_assignment',CASE WHEN pool_run_id IS NULL THEN NULL ELSE jsonb_build_object('run_id',pool_run_id,'role',pool_role) END,
            'desired_assignment',CASE WHEN desired_run_id IS NULL THEN NULL ELSE jsonb_build_object('run_id',desired_run_id,'role',desired_role) END,
            'current_assignment',CASE WHEN active_run_id IS NULL THEN NULL ELSE jsonb_build_object('run_id',active_run_id,'role',active_role) END)
            FROM nodes WHERE lease_expires_at>now() AND shutdown_requested_at IS NULL ORDER BY uuid FOR UPDATE"#)
            .fetch_all(&mut *tx).await?;
        let nodes = nodes
            .into_iter()
            .map(serde_json::from_value::<RegisteredNode>)
            .collect::<Result<Vec<_>, _>>()?;
        let existing = nodes
            .iter()
            .filter_map(|n| n.pool_assignment)
            .filter(|a| a.run_id == id)
            .fold((0, 0), |(s, e), a| match a.role {
                WorkerRole::SamplerAggregator => (s + 1, e),
                WorkerRole::Evaluator => (s, e + 1),
            });
        anyhow::ensure!(
            cap.is_none_or(|limit| existing.1 <= limit),
            "run already has more evaluators than the requested cap"
        );
        let (max_samplers, min_evaluators) = tasks
            .iter()
            .find(|t| t.state != crate::RunTaskState::Completed)
            .and_then(|t| match &t.task {
                crate::RunTaskSpec::IntegrationCampaign {
                    allocation,
                    children,
                    ..
                } => Some((
                    allocation.max_active_runs.min(children.len()),
                    allocation.min_evaluators_per_run.max(1),
                )),
                _ => None,
            })
            .unwrap_or((1, 1));
        if existing.0 > 0
            && existing.1 >= existing.0 * min_evaluators
            && nodes.iter().any(|n| {
                n.pool_assignment.is_some_and(|a| a.run_id == id) && n.desired_assignment.is_some()
            })
        {
            sqlx::query(RECORD_START).bind(id).execute(&mut *tx).await?;
            tx.commit().await?;
            return Ok(
                json!({"run_id":id,"start":{"state":"started","already_started":true,"assignments":[]}}),
            );
        }
        let free = nodes
            .iter()
            .filter(|n| {
                n.pool_assignment.is_none()
                    && n.desired_assignment.is_none()
                    && n.current_assignment.is_none()
            })
            .cloned()
            .collect::<Vec<_>>();
        let (samplers, evaluators) = crate::api::nodes::plan_auto_roles(
            &free,
            &spec.integration_params.sampler_requirements,
            &spec.integration_params.evaluator_requirements,
            existing,
            (max_samplers, min_evaluators),
            cap.unwrap_or(usize::MAX),
        );
        if existing.0 + samplers.len() == 0
            || existing.1 + evaluators.len() < (existing.0 + samplers.len()).max(1) * min_evaluators
        {
            return Ok(
                json!({"run_id":id,"start":{"state":"blocked","error":{"code":"workers_unavailable","retryable":true,
                "message":"Waiting for a complete compatible worker group to become idle; no workers were claimed."},
                "availability":{"live":nodes.len(),"idle":free.len(),
                "draining":nodes.iter().filter(|n|n.pool_assignment.is_none() && n.current_assignment.is_some()).count(),
                "compatible_idle_samplers":free.iter().filter(|n|crate::api::nodes::capabilities_satisfy(&n.capabilities,&spec.integration_params.sampler_requirements)).count(),
                "compatible_idle_evaluators":free.iter().filter(|n|crate::api::nodes::capabilities_satisfy(&n.capabilities,&spec.integration_params.evaluator_requirements)).count(),"assigned_samplers":existing.0,"assigned_evaluators":existing.1,
                "sampler_requirements":spec.integration_params.sampler_requirements,"evaluator_requirements":spec.integration_params.evaluator_requirements}}}),
            );
        }
        let mut assignments = Vec::new();
        for (selected, role) in [
            (&samplers, WorkerRole::SamplerAggregator),
            (&evaluators, WorkerRole::Evaluator),
        ] {
            for node in selected {
                sqlx::query("UPDATE nodes SET pool_run_id=$2,pool_role=$3,desired_run_id=$2,desired_role=$3,updated_at=now() WHERE uuid=$1")
                    .bind(&node.uuid).bind(id).bind(role.as_str()).execute(&mut *tx).await?;
                assignments.push(json!({"worker":node.name,"role":role}));
            }
        }
        sqlx::query("UPDATE nodes SET desired_run_id=pool_run_id,desired_role=pool_role,updated_at=now() WHERE pool_run_id=$1 AND desired_run_id IS NULL AND lease_expires_at>now() AND shutdown_requested_at IS NULL")
            .bind(id).execute(&mut *tx).await?;
        super::super::connection_budget::check_worker_connection_budget(&mut tx, 0).await?;
        sqlx::query(RECORD_START).bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(json!({"run_id":id,"start":{"state":"started","assignments":assignments}}))
    }
}
