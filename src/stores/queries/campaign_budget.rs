use std::collections::BTreeMap;

use sqlx::PgPool;

/// Allocate disjoint, durable portions of the campaign cap. A child may retain
/// its allowance through reassignment/recovery, but cannot borrow another's.
pub(crate) async fn allocate_campaign_samples(
    pool: &PgPool,
    task_id: i64,
    requests: &[(i32, i64)],
) -> Result<BTreeMap<i32, i64>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let (limit, active): (Option<i64>, bool) = sqlx::query_as(
        "SELECT (task->'stop_condition'->>'max_total_samples')::bigint, state='active'
         FROM run_tasks WHERE id=$1 AND task->>'kind'='integration_campaign' FOR UPDATE",
    )
    .bind(task_id)
    .fetch_one(&mut *tx)
    .await?;
    // Even error-only campaigns reserve bounded scheduling quanta.
    let limit = limit.unwrap_or(i64::MAX);
    // Seed existing children conservatively; never hide already produced work.
    sqlx::query(
        "INSERT INTO campaign_sample_budgets (child_run_id,campaign_task_id,granted_samples)
         SELECT r.id,$1,COALESCE(sum(t.nr_produced_samples),0)::bigint
         FROM runs r LEFT JOIN run_tasks t ON t.run_id=r.id WHERE r.parent_task_id=$1
         GROUP BY r.id ON CONFLICT (child_run_id) DO NOTHING",
    )
    .bind(task_id)
    .execute(&mut *tx)
    .await?;
    // Completed queues have drained and checkpointed. Their unused allowance
    // can safely be reused; active/paused children retain all their reservations.
    sqlx::query(
        "UPDATE campaign_sample_budgets b SET granted_samples=(
             SELECT COALESCE(sum(nr_produced_samples),0)::bigint FROM run_tasks WHERE run_id=b.child_run_id)
         WHERE b.campaign_task_id=$1 AND NOT EXISTS (
             SELECT 1 FROM run_tasks WHERE run_id=b.child_run_id AND state<>'completed')",
    ).bind(task_id).execute(&mut *tx).await?;
    let rows: Vec<(i32, i64)> = sqlx::query_as(
        "SELECT child_run_id,granted_samples FROM campaign_sample_budgets WHERE campaign_task_id=$1",
    ).bind(task_id).fetch_all(&mut *tx).await?;
    let mut grants = rows.into_iter().collect::<BTreeMap<_, _>>();
    let total = grants
        .values()
        .try_fold(0_i64, |total, n| total.checked_add(*n))
        .ok_or_else(|| sqlx::Error::Protocol("campaign sample accounting overflow".into()))?;
    if total > limit {
        return Err(sqlx::Error::Protocol(
            "campaign has already produced more than its sample cap".into(),
        ));
    }
    let mut remaining = limit - total;
    if active {
        for &(child, requested_total) in requests {
            let grant = grants.get_mut(&child).ok_or_else(|| {
                sqlx::Error::Protocol("budget request is not for a campaign child".into())
            })?;
            let extra = requested_total.saturating_sub(*grant).max(0).min(remaining);
            if extra == 0 {
                continue;
            }
            *grant += extra;
            remaining -= extra;
            sqlx::query(
                "UPDATE campaign_sample_budgets SET granted_samples=$2 WHERE child_run_id=$1",
            )
            .bind(child)
            .bind(*grant)
            .execute(&mut *tx)
            .await?;
        }
    }
    // A grant must survive a database restart before generation can use it.
    sqlx::query("SET LOCAL synchronous_commit = on")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(grants)
}

pub(crate) async fn campaign_sample_allowance(
    pool: &PgPool,
    run_id: i32,
    task_id: i64,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COALESCE(b.granted_samples,0) - COALESCE((
             SELECT sum(nr_produced_samples)::bigint FROM run_tasks WHERE run_id=$1 AND id<>$2),0)
         FROM runs r JOIN run_tasks parent ON parent.id=r.parent_task_id
         LEFT JOIN campaign_sample_budgets b ON b.child_run_id=r.id
         WHERE r.id=$1 AND parent.task->>'kind'='integration_campaign'
        ",
    )
    .bind(run_id)
    .bind(task_id)
    .fetch_optional(pool)
    .await
}
