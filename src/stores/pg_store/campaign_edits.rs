//! Validated campaign edits. All commits serialize with task activation/controller ticks.
use super::{
    PgStore,
    campaign::{campaign_task, lock_controller, save_task},
};
use crate::{
    api::{
        ApiError,
        runs::{self, task_definition, validate_queue_definition},
    },
    core::{
        IntegrationCampaignChildSpec, RunReadStore, RunTask, RunTaskInput, RunTaskSpec,
        RunTaskState, RunTaskStore, traits::TaskQueueChange,
    },
    stores::RunProgress,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChildEdit {
    pub action: String,
    #[serde(default)]
    pub toml: String,
    #[serde(default)]
    pub task_name: String,
    #[serde(default)]
    pub children: Vec<i32>,
    pub run_id: Option<i32>,
    #[serde(default)]
    pub label: String,
    #[serde(default = "unit")]
    pub coefficient: f64,
    pub expected: Option<Value>,
    pub request_id: Option<String>,
}
fn unit() -> f64 {
    1.0
}
fn bad(message: impl Into<String>) -> ApiError {
    ApiError::BadRequest(message.into())
}
fn definitions(tasks: &[RunTask]) -> Vec<RunTaskInput> {
    tasks.iter().map(task_definition).collect()
}
fn revision(tasks: &[RunTask]) -> Value {
    json!(
        tasks
            .iter()
            .map(|t| json!({"id":t.id.to_string(),"name":t.name,"state":t.state,"task":t.task}))
            .collect::<Vec<_>>()
    )
}
fn publishing(tasks: &[RunTaskInput]) -> bool {
    tasks.iter().any(|t| {
        matches!(
            t.task,
            RunTaskSpec::Sample {
                publish_result: true,
                ..
            }
        )
    })
}
fn changed_queue(
    run: &RunProgress,
    tasks: &[RunTask],
    edit: &ChildEdit,
) -> Result<Vec<RunTaskInput>, ApiError> {
    if run.archived {
        return Err(bad("archived result runs are read-only"));
    }
    let mut next = definitions(tasks);
    if edit.action == "add_task" {
        if tasks.iter().any(|t| t.state == RunTaskState::Failed) {
            return Err(bad("child run has failed"));
        }
        let addition = runs::parse_task_queue_toml(&edit.toml)?.into_tasks();
        if addition.len() != 1 {
            return Err(bad("provide exactly one task"));
        }
        if addition.iter().any(|task| task.task.is_controller()) {
            return Err(bad("controllers must be defined as run kinds"));
        }
        next.extend(addition);
    } else {
        let target = tasks
            .iter()
            .find(|t| t.name == edit.task_name)
            .ok_or_else(|| bad("task is absent"))?;
        if target.state != RunTaskState::Pending {
            return Err(bad("only pending tasks can be removed"));
        }
        next.retain(|t| t.name.as_deref() != Some(edit.task_name.as_str()));
    }
    if !publishing(&next) {
        return Err(bad("a child must retain a publishing sample task"));
    }
    validate_queue_definition(run, &next)?;
    Ok(next)
}

fn stable_preview(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("revision");
    }
    if let Some(source) = value.get_mut("source").and_then(Value::as_object_mut) {
        source.remove("produced_samples");
        source.remove("cpu_seconds");
        source.remove("workers");
    }
    // Progress may change between preview and confirmation. Definitions and
    // applicability remain part of the comparison.
    if let Some(children) = value.get_mut("children").and_then(Value::as_array_mut) {
        for child in children {
            if let Some(child) = child.as_object_mut() {
                child.remove("status");
            }
        }
    }
    value
}

fn preview_revision(value: &Value) -> Result<String, ApiError> {
    // Hash the server's representation, before the browser rewrites JSON
    // numbers (1.0 -> 1, or large integers rounded to JavaScript precision).
    // The opaque string survives that roundtrip without relaxing comparisons
    // of definitions, eligibility, or the requested edit.
    let bytes = serde_json::to_vec(&stable_preview(value))
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

impl PgStore {
    pub async fn campaign_children_editor(&self, run: i32, task: i64) -> Result<Value, ApiError> {
        let children = self
            .get_child_runs_for_task(run, task, "integration_campaign")
            .await?;
        let mut rows = Vec::new();
        let mut names = BTreeSet::new();
        let mut queues = Vec::new();
        for child in children {
            let tasks = self.list_run_tasks(child.run_id).await?;
            names.extend(tasks.iter().map(|t| t.name.clone()));
            rows.push(json!({"run_id":child.run_id,"label":child.spawn_label,"name":child.run_name,"status":child.lifecycle_state,"failed":tasks.iter().any(|t|t.state==RunTaskState::Failed)}));
            queues.push((child, tasks));
        }
        let mut task_names = Vec::new();
        for name in names {
            let edit = ChildEdit {
                action: "remove_task".into(),
                task_name: name.clone(),
                toml: String::new(),
                children: vec![],
                run_id: None,
                label: String::new(),
                coefficient: 1.,
                expected: None,
                request_id: None,
            };
            let count = queues
                .iter()
                .filter(|(run, tasks)| changed_queue(run, tasks, &edit).is_ok())
                .count();
            if count > 0 {
                task_names.push(json!({"name":name,"count":count}));
            }
        }
        let operations:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',id::text,'run_id',run_id,'operation',operation,'state',state,'error',error) FROM campaign_child_edits WHERE campaign_task_id=$1 ORDER BY id DESC LIMIT 20").bind(task).fetch_all(self.pool()).await.map_err(crate::core::StoreError::from)?;
        Ok(json!({"children":rows,"task_names":task_names,"operations":operations}))
    }

    pub async fn preview_campaign_edit(
        &self,
        run: i32,
        task: i64,
        edit: &ChildEdit,
    ) -> Result<Value, ApiError> {
        if self
            .get_run_progress(run)
            .await?
            .ok_or_else(|| bad("campaign run not found"))?
            .archived
        {
            return Err(bad("archived result runs are read-only"));
        }
        let parent = self
            .list_run_tasks(run)
            .await?
            .into_iter()
            .find(|t| t.id == task)
            .ok_or_else(|| bad("campaign task not found"))?;
        if !matches!(parent.state, RunTaskState::Pending | RunTaskState::Active) {
            return Err(bad("campaign is no longer editable"));
        }
        let RunTaskSpec::IntegrationCampaign {
            children,
            measurement,
            stop_condition,
            ..
        } = &parent.task
        else {
            return Err(bad("not an integration campaign"));
        };
        let mut result = json!({"campaign":parent.task,"children":[],"action":edit.action,"toml":edit.toml,"task_name":edit.task_name});
        if ["add_task", "remove_task"].contains(&edit.action.as_str()) {
            let mut rows = Vec::new();
            for child in self
                .get_child_runs_for_task(run, task, "integration_campaign")
                .await?
            {
                let tasks = self.list_run_tasks(child.run_id).await?;
                let error = changed_queue(&child, &tasks, edit)
                    .err()
                    .map(|e| e.to_string());
                rows.push(json!({"run_id":child.run_id,"label":child.spawn_label,"status":child.lifecycle_state,"applicable":error.is_none(),"reason":error,"revision":revision(&tasks)}));
            }
            result["children"] = json!(rows);
        } else if edit.action == "create" || edit.action == "adopt" {
            if children.iter().any(|c| c.name == edit.label) {
                return Err(bad("child label already exists"));
            }
            let text = if edit.action == "adopt" {
                let id = edit.run_id.ok_or_else(|| bad("select a run"))?;
                let source = self
                    .get_run_progress(id)
                    .await?
                    .ok_or_else(|| bad("run not found"))?;
                if source.archived {
                    return Err(bad("archived result runs cannot be adopted"));
                }
                if id == run || source.parent_run_id.is_some() || source.kind() != "integration" {
                    return Err(bad("select a standalone integration run"));
                }
                let pending_launch:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM nodes WHERE pool_run_id=$1 AND worker_launch_pending(name,launch_request_id,last_seen))").bind(id).fetch_one(self.pool()).await.map_err(crate::core::StoreError::from)?;
                if pending_launch {
                    return Err(bad(
                        "run has a pending worker launch; wait for it before adoption",
                    ));
                }
                let tasks = self.list_run_tasks(id).await?;
                if source.active_worker_count == 0 && source.desired_assignment_count == 0 {
                    use crate::core::AggregationStore;
                    if source.pending_batches > 0
                        || source.claimed_batches > 0
                        || self.load_sampler_checkpoint(id).await?.is_some_and(|c| {
                            c.runtime_state.generation.has_pending()
                                || c.runtime_state.generation.has_outstanding()
                        })
                    {
                        return Err(bad(
                            "paused run still has outstanding samples; resume it so they can drain before adoption",
                        ));
                    }
                }

                let spent: i64 = tasks.iter().map(|t| t.nr_produced_samples).sum();
                let already:i64=sqlx::query_scalar("SELECT COALESCE(sum(nr_produced_samples),0)::bigint FROM run_tasks WHERE run_id IN (SELECT id FROM runs WHERE parent_task_id=$1)").bind(task).fetch_one(self.pool()).await.map_err(crate::core::StoreError::from)?;
                if stop_condition
                    .max_total_samples
                    .is_some_and(|cap| spent.saturating_add(already) > cap)
                {
                    return Err(bad(
                        "existing samples exceed the campaign cap; raise the cap before adopting this run",
                    ));
                }
                let overlap:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM run_tasks a JOIN run_tasks b ON a.spawned_from_snapshot_id=b.spawned_from_snapshot_id WHERE a.run_id=$1 AND b.run_id IN(SELECT id FROM runs WHERE parent_task_id=$2) AND a.spawned_from_snapshot_id IS NOT NULL AND NOT COALESCE(a.task->'accumulator' ? 'config',false) AND NOT COALESCE(b.task->'accumulator' ? 'config',false))").bind(id).bind(task).fetch_one(self.pool()).await.map_err(crate::core::StoreError::from)?;
                if overlap {
                    return Err(bad(
                        "run shares checkpoint ancestry with a campaign child; independent contributions cannot be established",
                    ));
                }
                result["source"] = json!({"run_id":id,"name":source.run_name,"revision":revision(&tasks),"produced_samples":spent,"cpu_seconds":source.cpu_seconds,"workers":source.desired_assignment_count.max(source.active_worker_count)});
                runs::export_run_definition(self, id).await?
            } else {
                edit.toml.clone()
            };
            let config = runs::parse_run_add_config_toml(&text)?;
            runs::validate_run(config.clone(), false)?;
            let schema = runs::campaign_component_schema(config.clone())?;
            if let Some(reference) = children.first() {
                let reference = crate::api::run_definition::instantiate_child(
                    reference.run.clone(),
                    reference.replacements.clone(),
                    None,
                )?;
                let expected = runs::campaign_component_schema(reference)?;
                if schema != expected {
                    return Err(bad(format!(
                        "components {:?} do not match campaign components {:?}",
                        schema, expected
                    )));
                }
            }
            let prepared = crate::preprocess::prepare_run(config)?;
            if prepared.kind != "integration" || !publishing(&prepared.task_queue) {
                return Err(bad(
                    "child must be an integration run with a publishing sample task",
                ));
            }
            // Resolve each publishing stage's configured projection before adoption.
            // validate_run checks stage sources/domains; actual component projections
            // are checked below when published data exists.
            if let Some(id) = edit.run_id.filter(|_| edit.action == "adopt") {
                let (saved, _, _) =
                    crate::runners::controller_child::load_published_child_result(self, id).await?;
                if let Some(accumulator) = saved.accumulator {
                    crate::services::measurement::project_measurement_results(
                        &accumulator,
                        measurement,
                        None,
                        &saved.source_task,
                    )?;
                }
            }
            let child = IntegrationCampaignChildSpec {
                name: edit.label.clone(),
                coefficient: edit.coefficient,
                run: crate::core::tasks::ChildRunSource::Inline(text),
                replacements: BTreeMap::new(),
            };
            child.validate().map_err(bad)?;
            result["child"] = json!(child);
        } else if edit.action == "delete" {
            let id = edit.run_id.ok_or_else(|| bad("select a child"))?;
            let child = self
                .get_run_progress(id)
                .await?
                .filter(|r| {
                    r.parent_run_id == Some(run)
                        && r.parent_task_id.as_deref() == Some(&task.to_string())
                })
                .ok_or_else(|| bad("run is not a child of this campaign task"))?;
            if children.len() <= 1 {
                return Err(bad("cannot delete the last campaign child"));
            }
            result["source"] = json!({"run_id":id,"label":child.spawn_label,"name":child.run_name});
        } else {
            return Err(bad("unknown edit action"));
        }
        result["revision"] = json!(preview_revision(&result)?);
        Ok(result)
    }

    pub async fn apply_campaign_edit(
        &self,
        run: i32,
        task: i64,
        edit: ChildEdit,
    ) -> Result<Value, ApiError> {
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(crate::core::StoreError::from)?;
        lock_controller(&mut tx).await?;
        let request_id = edit
            .request_id
            .as_deref()
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .ok_or_else(|| bad("request_id is required"))?;
        let request = json!(edit);
        let receipt:Option<(Value,Value)>=sqlx::query_as("SELECT request,response FROM campaign_edit_receipts WHERE campaign_task_id=$1 AND request_id=$2").bind(task).bind(request_id).fetch_optional(&mut *tx).await.map_err(crate::core::StoreError::from)?;
        if let Some((previous, response)) = receipt {
            if previous != request {
                return Err(bad("request_id already used"));
            }
            return Ok(response);
        }
        let preview = self.preview_campaign_edit(run, task, &edit).await?;
        if edit
            .expected
            .as_ref()
            .and_then(|expected| expected.get("revision"))
            != preview.get("revision")
        {
            return Err(bad(
                "children or task definitions changed; review a fresh preview before applying",
            ));
        }
        let (name, mut spec) = campaign_task(&mut tx, run, task).await?;
        let response = if ["add_task", "remove_task"].contains(&edit.action.as_str()) {
            let selected = edit.children.iter().copied().collect::<BTreeSet<_>>();
            if selected.is_empty() {
                return Err(bad("select at least one applicable child"));
            }
            let mut changed = Vec::new();
            for id in selected {
                if !preview["children"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r["run_id"] == id && r["applicable"] == true)
                {
                    return Err(bad("selected child is no longer applicable"));
                }
                let tasks = self.list_run_tasks(id).await?;
                if preview["children"]
                    .as_array()
                    .and_then(|rows| rows.iter().find(|r| r["run_id"] == id))
                    .and_then(|r| r.get("revision"))
                    != Some(&revision(&tasks))
                {
                    return Err(bad("child task state changed; review a fresh preview"));
                }
                let change = if edit.action == "add_task" {
                    TaskQueueChange::Append(runs::parse_task_queue_toml(&edit.toml)?.into_tasks())
                } else {
                    TaskQueueChange::Remove {
                        task_id: tasks.iter().find(|t| t.name == edit.task_name).unwrap().id,
                    }
                };
                super::queries::apply_task_queue_change_in_transaction(&mut tx, id, &tasks, change)
                    .await?;
                changed.push(id);
            }
            json!({"updated":changed})
        } else if edit.action == "create" {
            let RunTaskSpec::IntegrationCampaign { children, .. } = &mut spec else {
                unreachable!()
            };
            children.push(
                serde_json::from_value(preview["child"].clone()).map_err(|e| bad(e.to_string()))?,
            );
            save_task(&mut tx, task, name, spec).await?;
            json!({"created":edit.label})
        } else {
            let id = edit.run_id.unwrap();
            let operation = if edit.action == "delete" {
                "delete"
            } else {
                "adopt"
            };
            sqlx::query("INSERT INTO campaign_child_edits(campaign_task_id,run_id,operation,payload) VALUES($1,$2,$3,$4)").bind(task).bind(id).bind(operation).bind(&preview).execute(&mut *tx).await.map_err(crate::core::StoreError::from)?;
            sqlx::query("INSERT INTO run_drain_requests(run_id,acknowledged) SELECT $1,NOT EXISTS(SELECT 1 FROM nodes WHERE (active_run_id=$1 OR desired_run_id=$1) AND lease_expires_at>now()) ON CONFLICT DO NOTHING").bind(id).execute(&mut *tx).await.map_err(crate::core::StoreError::from)?;
            json!({"queued":operation,"run_id":id})
        };
        sqlx::query("UPDATE run_tasks SET controller_output=NULL WHERE id=$1")
            .bind(task)
            .execute(&mut *tx)
            .await
            .map_err(crate::core::StoreError::from)?;
        sqlx::query("INSERT INTO campaign_edit_receipts(campaign_task_id,request_id,request,response) VALUES($1,$2,$3,$4)").bind(task).bind(request_id).bind(request).bind(&response).execute(&mut *tx).await.map_err(crate::core::StoreError::from)?;
        super::queries::insert_task_output_snapshot(&mut *tx,run,task,&json!({"sources":[],"metrics":[],"observables":null,"coverage":{"partial":true,"editing":true}})).await.map_err(crate::core::StoreError::from)?;
        tx.commit().await.map_err(crate::core::StoreError::from)?;
        Ok(response)
    }

    /// Called under the controller lock. Waiting never holds a transaction open.
    pub(crate) async fn reconcile_campaign_controls(&self) -> Result<(), crate::core::StoreError> {
        sqlx::query("UPDATE run_drain_requests d SET acknowledged=true WHERE NOT EXISTS(SELECT 1 FROM run_tasks WHERE run_id=d.run_id AND state='active') AND NOT EXISTS(SELECT 1 FROM batches WHERE run_id=d.run_id AND status IN ('pending','claimed'))").execute(self.pool()).await?;
        // Failed tasks cannot acknowledge a drain. Their workers first release
        // placements; retain the last published data rather than restarting them.
        sqlx::query("UPDATE run_drain_requests d SET acknowledged=true WHERE EXISTS(SELECT 1 FROM run_tasks WHERE run_id=d.run_id AND state='failed') AND NOT EXISTS(SELECT 1 FROM nodes WHERE active_run_id=d.run_id AND lease_expires_at>now())").execute(self.pool()).await?;
        // Adoption/deletion releases the old runtime before changing parentage.
        sqlx::query("UPDATE nodes n SET desired_run_id=NULL,desired_role=NULL WHERE EXISTS(SELECT 1 FROM campaign_child_edits e JOIN run_drain_requests d ON d.run_id=e.run_id WHERE e.state='pending' AND d.acknowledged AND n.desired_run_id=e.run_id)").execute(self.pool()).await?;
        let edits:Vec<(i64,i64,i32,String,Value)>=sqlx::query_as("SELECT e.id,e.campaign_task_id,e.run_id,e.operation,e.payload FROM campaign_child_edits e JOIN run_drain_requests d ON d.run_id=e.run_id WHERE e.state='pending' AND d.acknowledged AND NOT EXISTS(SELECT 1 FROM nodes WHERE active_run_id=e.run_id AND lease_expires_at>now()) ORDER BY e.id").fetch_all(self.pool()).await?;
        for (id, task, child, operation, payload) in edits {
            if let Err(error) = self
                .finish_campaign_child_edit(id, task, child, &operation, payload)
                .await
            {
                if error.is_database_error() {
                    return Err(error);
                }
                sqlx::query("UPDATE campaign_child_edits SET state='failed',error=$2 WHERE id=$1")
                    .bind(id)
                    .bind(error.to_string())
                    .execute(self.pool())
                    .await?;
                sqlx::query("DELETE FROM run_drain_requests WHERE run_id=$1")
                    .bind(child)
                    .execute(self.pool())
                    .await?;
            }
        }
        // Reclaim ungenerated allowance only after every affected child has
        // acknowledged. This prevents old reservations receiving new work early.
        let ready:Vec<i64>=sqlx::query_scalar("SELECT DISTINCT r.parent_task_id FROM runs r JOIN run_drain_requests d ON d.run_id=r.id WHERE r.spawn_kind='integration_campaign' AND d.acknowledged AND NOT EXISTS(SELECT 1 FROM run_drain_requests other JOIN runs c ON c.id=other.run_id WHERE c.parent_task_id=r.parent_task_id AND NOT other.acknowledged) AND NOT EXISTS(SELECT 1 FROM campaign_child_edits e WHERE e.campaign_task_id=r.parent_task_id AND e.state='pending')").fetch_all(self.pool()).await?;
        for task in ready {
            let mut tx = self.pool().begin().await?;
            sqlx::query("UPDATE campaign_sample_budgets b SET granted_samples=(SELECT COALESCE(sum(nr_produced_samples),0)::bigint FROM run_tasks WHERE run_id=b.child_run_id) WHERE b.campaign_task_id=$1 AND EXISTS(SELECT 1 FROM run_drain_requests WHERE run_id=b.child_run_id AND acknowledged)").bind(task).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM run_drain_requests WHERE run_id IN(SELECT id FROM runs WHERE parent_task_id=$1) AND acknowledged").bind(task).execute(&mut *tx).await?;
            sqlx::query("UPDATE run_tasks SET controller_output=NULL WHERE id=$1")
                .bind(task)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        Ok(())
    }

    async fn finish_campaign_child_edit(
        &self,
        id: i64,
        task: i64,
        child: i32,
        operation: &str,
        payload: Value,
    ) -> Result<(), crate::core::StoreError> {
        let mut tx = self.pool().begin().await?;
        let parent: i32 = sqlx::query_scalar("SELECT run_id FROM run_tasks WHERE id=$1")
            .bind(task)
            .fetch_one(&mut *tx)
            .await?;
        let (name, mut spec) = campaign_task(&mut tx, parent, task).await?;
        sqlx::query("SELECT name FROM nodes WHERE pool_run_id=$1 OR active_run_id=$1 OR desired_run_id=$1 FOR UPDATE").bind(child).fetch_all(&mut *tx).await?;
        let busy:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM nodes WHERE desired_run_id=$1 OR (active_run_id=$1 AND lease_expires_at>now()))").bind(child).fetch_one(&mut *tx).await?;
        if busy {
            return Ok(());
        }

        let RunTaskSpec::IntegrationCampaign { children, .. } = &mut spec else {
            unreachable!()
        };
        sqlx::query("SELECT id FROM runs WHERE id=$1 FOR UPDATE")
            .bind(child)
            .fetch_one(&mut *tx)
            .await?;
        if operation == "delete" {
            let label: String = sqlx::query_scalar(
                "SELECT spawn_label FROM runs WHERE id=$1 AND parent_task_id=$2",
            )
            .bind(child)
            .bind(task)
            .fetch_one(&mut *tx)
            .await?;
            children.retain(|c| c.name != label);
            if children.is_empty() {
                return Err(crate::core::StoreError::invalid_input(
                    "cannot remove the last child",
                ));
            }
            sqlx::query("INSERT INTO campaign_removed_work SELECT $1,$2,$3,COALESCE(sum(nr_produced_samples),0)::bigint,COALESCE(sum(nr_completed_samples),0)::bigint,COALESCE(sum(cpu_seconds),0)::double precision FROM run_tasks WHERE run_id=$2 ON CONFLICT DO NOTHING").bind(task).bind(child).bind(&label).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM campaign_queue_tuning_rules WHERE campaign_task_id=$1 AND child_label=$2").bind(task).bind(label).execute(&mut *tx).await?;
            sqlx::query("UPDATE run_tasks t SET nr_produced_samples=t.nr_produced_samples+w.produced_samples,nr_completed_samples=t.nr_completed_samples+w.completed_samples,cpu_seconds=t.cpu_seconds+w.cpu_seconds FROM campaign_removed_work w WHERE t.id=$1 AND w.campaign_task_id=t.id AND w.child_run_id=$2").bind(task).bind(child).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM runs WHERE id=$1")
                .bind(child)
                .execute(&mut *tx)
                .await?;
        } else {
            let member: IntegrationCampaignChildSpec =
                serde_json::from_value(payload["child"].clone())?;
            if children.iter().any(|c| c.name == member.name) {
                return Err(crate::core::StoreError::invalid_input(
                    "child label is already in use",
                ));
            }
            let updated=sqlx::query("UPDATE runs SET parent_run_id=$2,parent_task_id=$3,spawn_kind='integration_campaign',spawn_label=$4 WHERE id=$1 AND parent_run_id IS NULL").bind(child).bind(parent).bind(task).bind(&member.name).execute(&mut *tx).await?;
            if updated.rows_affected() != 1 {
                return Err(crate::core::StoreError::invalid_input(
                    "run is already managed by another parent",
                ));
            }
            sqlx::query("UPDATE nodes SET pool_run_id=$2,desired_run_id=CASE WHEN lease_expires_at>now() THEN $2 ELSE NULL END,desired_role=CASE WHEN lease_expires_at>now() THEN pool_role ELSE NULL END WHERE pool_run_id=$1").bind(child).bind(parent).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO campaign_sample_budgets(child_run_id,campaign_task_id,granted_samples) SELECT $1,$2,COALESCE(sum(nr_produced_samples),0)::bigint FROM run_tasks WHERE run_id=$1").bind(child).bind(task).execute(&mut *tx).await?;
            children.push(member);
        }
        save_task(&mut tx, task, name, spec).await?;
        sqlx::query("DELETE FROM run_drain_requests WHERE run_id=$1")
            .bind(child)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE campaign_child_edits SET state='completed' WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE run_tasks SET controller_output=NULL WHERE id=$1")
            .bind(task)
            .execute(&mut *tx)
            .await?;
        super::queries::insert_task_output_snapshot(&mut *tx,parent,task,&json!({"sources":[],"metrics":[],"observables":null,"coverage":{"partial":true,"editing":true}})).await?;
        tx.commit().await?;
        Ok(())
    }
}
