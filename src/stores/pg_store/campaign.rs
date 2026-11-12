//! Campaign edits serialize with the controller and preserve existing reservations.
use super::PgStore;
use crate::core::{
    IntegrationCampaignAllocationSpec, IntegrationCampaignStopCondition, RunTaskInput, RunTaskSpec,
    SamplerQueueTuning, StoreError, canonical_task_toml,
};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};

async fn campaign_task(
    tx: &mut Transaction<'_, Postgres>,
    run: i32,
    task: i64,
) -> Result<(String, RunTaskSpec), StoreError> {
    let row: Option<(String, Value, String)> = sqlx::query_as(
        "SELECT name,task,state FROM run_tasks WHERE id=$1 AND run_id=$2 FOR UPDATE",
    )
    .bind(task)
    .bind(run)
    .fetch_optional(&mut **tx)
    .await?;
    let (name, spec, state) =
        row.ok_or_else(|| StoreError::NotFound("Campaign task not found".into()))?;
    if !["pending", "active"].contains(&state.as_str()) {
        return Err(StoreError::invalid_input(
            "Only pending or active campaigns can be changed",
        ));
    }
    let spec: RunTaskSpec = serde_json::from_value(spec)?;
    if !matches!(spec, RunTaskSpec::IntegrationCampaign { .. }) {
        return Err(StoreError::invalid_input(
            "Task is not an integration campaign",
        ));
    }
    Ok((name, spec))
}

async fn lock_controller(tx: &mut Transaction<'_, Postgres>) -> Result<(), StoreError> {
    sqlx::query("SET LOCAL lock_timeout='5s'")
        .execute(&mut **tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(1196245588, 1)")
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub fn settings(spec: &RunTaskSpec) -> Value {
    match spec {
        RunTaskSpec::IntegrationCampaign {
            allocation,
            stop_condition,
            ..
        } => json!({"allocation":allocation,"stop_condition":stop_condition}),
        _ => Value::Null,
    }
}

impl PgStore {
    /// Allocated core-hours of result-publishing sample tasks, including live
    /// worker accounting that has not yet been folded into the task baseline.
    pub async fn campaign_publishing_core_hours(
        &self,
        run: i32,
        task: i64,
    ) -> Result<std::collections::BTreeMap<i32, f64>, StoreError> {
        let rows: Vec<(i32, f64)> = sqlx::query_as(
            r#"
            SELECT r.id, COALESCE(sum(task_cpu_seconds(t.id,t.cpu_seconds)) FILTER (
                WHERE t.task->>'kind'='sample' AND COALESCE((t.task->>'publish_result')::bool,true)
            ),0.0)::double precision / 3600.0
            FROM runs r LEFT JOIN run_tasks t ON t.run_id=r.id
            WHERE r.parent_run_id=$1 AND r.parent_task_id=$2 AND r.spawn_kind='integration_campaign'
            GROUP BY r.id
        "#,
        )
        .bind(run)
        .bind(task)
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().collect())
    }

    pub async fn update_campaign_scheduling(
        &self,
        run: i32,
        task: i64,
        expected: Value,
        allocation: IntegrationCampaignAllocationSpec,
        stop_condition: IntegrationCampaignStopCondition,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool().begin().await?;
        lock_controller(&mut tx).await?;
        let (name, mut spec) = campaign_task(&mut tx, run, task).await?;
        if settings(&spec) != expected {
            return Err(StoreError::invalid_input(
                "Scheduling settings changed since this form was opened. Reload before applying.",
            ));
        }
        if let Some(cap) = stop_condition.max_total_samples {
            let reserved: i64 = sqlx::query_scalar("SELECT COALESCE(sum(GREATEST(COALESCE(b.granted_samples,0), COALESCE((SELECT sum(t.nr_produced_samples) FROM run_tasks t WHERE t.run_id=r.id),0))),0)::bigint FROM runs r LEFT JOIN campaign_sample_budgets b ON b.child_run_id=r.id WHERE r.parent_task_id=$1")
                .bind(task).fetch_one(&mut *tx).await?;
            if cap < reserved {
                return Err(StoreError::invalid_input(format!(
                    "Sample cap must cover {reserved} already produced or reserved samples"
                )));
            }
        }
        if let RunTaskSpec::IntegrationCampaign {
            allocation: a,
            stop_condition: s,
            ..
        } = &mut spec
        {
            *a = allocation;
            *s = stop_condition;
        }
        spec.validate().map_err(StoreError::invalid_input)?;
        save_task(&mut tx, task, name, spec).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn update_campaign_queue_tuning(
        &self,
        run: i32,
        task: i64,
        labels: &[String],
        scope: &str,
        patch: Value,
    ) -> Result<Value, StoreError> {
        validate_patch(&patch)?;
        if labels.is_empty() {
            return Err(StoreError::invalid_input("Select at least one child"));
        }
        if !["all", "publishing", "training"].contains(&scope) && !scope.starts_with("name:") {
            return Err(StoreError::invalid_input("Unknown task scope"));
        }
        let mut tx = self.pool().begin().await?;
        lock_controller(&mut tx).await?;
        let (_, spec) = campaign_task(&mut tx, run, task).await?;
        let RunTaskSpec::IntegrationCampaign { children, .. } = spec else {
            unreachable!()
        };
        for label in labels {
            if !children.iter().any(|child| child.name == *label) {
                return Err(StoreError::invalid_input(format!(
                    "Unknown campaign child {label}"
                )));
            }
            // Store precedence per field: editing a duration must not reapply an
            // older batch-size rule over a newer stage-specific override.
            for (field, value) in patch.as_object().unwrap() {
                let single = Value::Object([(field.clone(), value.clone())].into_iter().collect());
                sqlx::query("INSERT INTO campaign_queue_tuning_rules(campaign_task_id,child_label,task_scope,field_name,patch) VALUES($1,$2,$3,$4,$5) ON CONFLICT(campaign_task_id,child_label,task_scope,field_name) DO UPDATE SET patch=EXCLUDED.patch, updated_at=clock_timestamp()")
                    .bind(task).bind(label).bind(scope).bind(field).bind(single).execute(&mut *tx).await?;
            }
        }
        let result = apply_rules(&mut tx, task, Some(labels), Some(scope)).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub(crate) async fn apply_campaign_tuning_rules(&self, task: i64) -> Result<(), StoreError> {
        // Most controller ticks have nothing to apply. Read only small task/rule
        // fields first, avoiding task-row locks and JSON decoding on every tick.
        // This also catches a crash between child creation and rule application.
        let changed: bool = sqlx::query_scalar(r#"
            WITH expected AS (
                SELECT DISTINCT ON (t.id,q.field_name) t.id,q.field_name,
                    COALESCE(t.task->'queue_tuning'->q.field_name,'null'::jsonb) AS current,
                    q.patch->q.field_name AS desired
                FROM campaign_queue_tuning_rules q JOIN runs r ON r.parent_task_id=q.campaign_task_id AND r.spawn_label=q.child_label
                JOIN run_tasks t ON t.run_id=r.id
                WHERE q.campaign_task_id=$1 AND t.state IN ('pending','active') AND t.task->>'kind'='sample'
                    AND (q.task_scope='all' OR (q.task_scope='publishing' AND COALESCE((t.task->>'publish_result')::bool,true))
                        OR (q.task_scope='training' AND NOT COALESCE((t.task->>'publish_result')::bool,true)) OR q.task_scope='name:'||t.name)
                ORDER BY t.id,q.field_name,q.updated_at DESC,q.id DESC
            ) SELECT EXISTS(SELECT 1 FROM expected WHERE current IS DISTINCT FROM desired)
        "#).bind(task).fetch_one(self.pool()).await?;
        if !changed {
            return Ok(());
        }
        let mut tx = self.pool().begin().await?;
        apply_rules(&mut tx, task, None, None).await?;
        tx.commit().await?;
        Ok(())
    }
}

fn validate_patch(patch: &Value) -> Result<(), StoreError> {
    let Some(fields) = patch.as_object().filter(|fields| !fields.is_empty()) else {
        return Err(StoreError::invalid_input(
            "Change at least one queue setting",
        ));
    };
    for (key, value) in fields {
        if ![
            "target_batch_eval_ms",
            "max_generation_size",
            "max_batch_size",
            "fixed_batch_size",
        ]
        .contains(&key.as_str())
            || (value.is_null() && key != "fixed_batch_size")
        {
            return Err(StoreError::invalid_input(format!(
                "Invalid queue field {key}"
            )));
        }
    }
    let tuning: SamplerQueueTuning = serde_json::from_value(patch.clone())
        .map_err(|e| StoreError::invalid_input(e.to_string()))?;
    tuning.validate().map_err(StoreError::invalid_input)
}

fn matches_scope(spec: &RunTaskSpec, name: &str, scope: &str) -> bool {
    let RunTaskSpec::Sample { publish_result, .. } = spec else {
        return false;
    };
    scope == "all"
        || (scope == "publishing" && *publish_result)
        || (scope == "training" && !publish_result)
        || scope.strip_prefix("name:") == Some(name)
}

type TuningTask = (i64, String, Value, String, String, Value);
async fn apply_rules(
    tx: &mut Transaction<'_, Postgres>,
    task: i64,
    selected: Option<&[String]>,
    scope: Option<&str>,
) -> Result<Value, StoreError> {
    let rules: Vec<(String, String, Value)> = sqlx::query_as("SELECT child_label,task_scope,patch FROM campaign_queue_tuning_rules WHERE campaign_task_id=$1 ORDER BY updated_at,id")
        .bind(task).fetch_all(&mut **tx).await?;
    if rules.is_empty() {
        return Ok(json!({"updated":[],"skipped":[]}));
    }
    let rows: Vec<TuningTask> = sqlx::query_as("SELECT t.id,t.name,t.task,t.state,r.spawn_label,r.integration_params FROM run_tasks t JOIN runs r ON r.id=t.run_id WHERE r.parent_task_id=$1 ORDER BY t.id FOR UPDATE OF t")
        .bind(task).fetch_all(&mut **tx).await?;
    let mut updated = vec![];
    let mut skipped = vec![];
    for (id, name, value, state, label, params) in rows {
        let mut spec: RunTaskSpec = serde_json::from_value(value)?;
        if !matches!(spec, RunTaskSpec::Sample { .. }) {
            continue;
        }
        let reported = selected.is_some_and(|labels| labels.contains(&label))
            && scope.is_some_and(|s| matches_scope(&spec, &name, s));
        if !["pending", "active"].contains(&state.as_str()) {
            if reported {
                skipped.push(json!({"child":label,"task":name,"state":state}));
            }
            continue;
        }
        let original = spec.sample_queue_tuning().cloned().unwrap_or_default();
        let mut next = serde_json::to_value(&original)?;
        for (child, scope, patch) in &rules {
            if *child == label && matches_scope(&spec, &name, scope) {
                for (key, value) in patch.as_object().unwrap() {
                    next[key] = value.clone();
                }
            }
        }
        let next: SamplerQueueTuning = serde_json::from_value(next)?;
        next.validate().map_err(StoreError::invalid_input)?;
        // Validate against effective defaults as well as the edited subset.
        let mut effective = params
            .pointer("/sampler_aggregator_runner_params/queue")
            .cloned()
            .unwrap_or(json!({}));
        for (key, value) in serde_json::to_value(&next)?.as_object().unwrap() {
            if !value.is_null() {
                effective[key] = value.clone();
            }
        }
        if effective["fixed_batch_size"]
            .as_u64()
            .zip(effective["max_batch_size"].as_u64())
            .is_some_and(|(fixed, max)| fixed > max)
        {
            return Err(StoreError::invalid_input(format!(
                "{label}/{name}: fixed batch size exceeds the maximum"
            )));
        }
        if next != original {
            spec.set_sample_queue_tuning(Some(next))
                .map_err(StoreError::invalid_input)?;
            spec.validate().map_err(StoreError::invalid_input)?;
            save_task(tx, id, name.clone(), spec).await?;
        }
        if reported {
            updated.push(json!({"child":label,"task":name,"state":state}));
        }
    }
    Ok(json!({"updated":updated,"skipped":skipped}))
}

async fn save_task(
    tx: &mut Transaction<'_, Postgres>,
    id: i64,
    name: String,
    spec: RunTaskSpec,
) -> Result<(), StoreError> {
    let toml = canonical_task_toml(&RunTaskInput {
        name: Some(name),
        task: spec.clone(),
    })
    .map_err(|e| StoreError::invalid_input(e.to_string()))?;
    sqlx::query("UPDATE run_tasks SET task=$2,task_toml=$3 WHERE id=$1")
        .bind(id)
        .bind(serde_json::to_value(spec)?)
        .bind(toml)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
