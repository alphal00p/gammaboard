use super::{PgStore, TaskOutputSnapshot};
use serde_json::Value;

#[derive(sqlx::FromRow)]
struct PlotSnapshot {
    id: i64,
    run_id: i32,
    task_id: i64,
    persisted_output: Value,
    created_at: chrono::DateTime<chrono::Utc>,
    sampler_uptime_ms: Option<f64>,
}

impl From<PlotSnapshot> for TaskOutputSnapshot {
    fn from(row: PlotSnapshot) -> Self {
        Self {
            id: row.id.to_string(),
            run_id: row.run_id,
            task_id: row.task_id.to_string(),
            persisted_output: row.persisted_output,
            created_at: Some(row.created_at),
            sampler_uptime_ms: row.sampler_uptime_ms,
        }
    }
}

pub(crate) struct PlotHistoryPage {
    pub snapshots: Vec<TaskOutputSnapshot>,
    pub head: i64,
    pub reset: bool,
}

impl PgStore {
    pub(crate) async fn current_plot_observable(
        &self,
        run: i32,
    ) -> Result<Option<Value>, sqlx::Error> {
        sqlx::query_scalar("SELECT COALESCE(current_plot_observable,gammaboard_plot_observable(current_observable),current_observable) FROM runs WHERE id=$1")
            .bind(run).fetch_optional(self.pool()).await.map(Option::flatten)
    }

    pub(crate) async fn latest_plot_snapshot(
        &self,
        run: i32,
        task: i64,
    ) -> Result<Option<TaskOutputSnapshot>, sqlx::Error> {
        sqlx::query_as::<_,PlotSnapshot>("SELECT id,run_id,task_id,COALESCE(plot_observable,gammaboard_plot_observable(persisted_observable),persisted_observable) AS persisted_output,created_at,NULL::double precision AS sampler_uptime_ms FROM persisted_observable_snapshots WHERE run_id=$1 AND task_id=$2 ORDER BY id DESC LIMIT 1")
            .bind(run).bind(task).fetch_optional(self.pool()).await.map(|r| r.map(Into::into))
    }

    /// Sample IDs before touching JSON payloads. Keep both ends of the complete
    /// available history, and reset after rollback or a long gap between visits.
    pub(crate) async fn plot_history(
        &self,
        run: i32,
        task: i64,
        cursor: Option<i64>,
        limit: i64,
    ) -> Result<PlotHistoryPage, sqlx::Error> {
        let limit = limit.clamp(2, 256);
        let (head, cursor_exists, delta): (i64,bool,i64) = sqlx::query_as(r#"
            SELECT COALESCE(max(id),0), COALESCE(bool_or(id=$3),false), count(*) FILTER(WHERE id>COALESCE($3,0))
            FROM persisted_observable_snapshots WHERE run_id=$1 AND task_id=$2
        "#).bind(run).bind(task).bind(cursor).fetch_one(self.pool()).await?;
        let reset =
            cursor.is_none() || cursor.is_some_and(|id| id != 0 && !cursor_exists) || delta > limit;
        let after = if reset { 0 } else { cursor.unwrap_or(0) };
        let rows = sqlx::query_as::<_,PlotSnapshot>(r#"
            WITH ranked AS MATERIALIZED (
                SELECT id,row_number() OVER(ORDER BY id) AS rn,count(*) OVER() AS total
                FROM persisted_observable_snapshots WHERE run_id=$1 AND task_id=$2 AND id>$3 AND id<=$4
            ), chosen AS MATERIALIZED (
                SELECT id FROM ranked WHERE rn=1 OR rn=total OR mod(rn-1,GREATEST(1,ceil((total-1)::numeric/($5-1))::bigint))=0
            )
            SELECT s.id,s.run_id,s.task_id,COALESCE(s.plot_observable,gammaboard_plot_observable(s.persisted_observable),s.persisted_observable) AS persisted_output,s.created_at,
                (SELECT (p.metrics->>'sampler_uptime_ms')::double precision FROM sampler_aggregator_performance_history p
                 WHERE p.run_id=s.run_id AND p.created_at<=s.created_at AND p.metrics->>'task_id'=s.task_id::text
                 ORDER BY p.created_at DESC,p.id DESC LIMIT 1) AS sampler_uptime_ms
            FROM chosen JOIN persisted_observable_snapshots s USING(id) ORDER BY s.id DESC
        "#).bind(run).bind(task).bind(after).bind(head).bind(limit).fetch_all(self.pool()).await?;
        Ok(PlotHistoryPage {
            snapshots: rows.into_iter().map(Into::into).collect(),
            head,
            reset,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::AccumulatorState;

    #[tokio::test]
    #[ignore = "requires an isolated migrated postgres database"]
    async fn compact_history_covers_both_ends_and_resets_after_recovery() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&std::env::var("GAMMABOARD_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
        let store = PgStore::new(pool);
        let excluded: bool = sqlx::query_scalar("SELECT gammaboard_plot_observable('{\"kind\":\"full_vector\",\"values\":[1,2,3]}') IS NULL")
            .fetch_one(store.pool()).await.unwrap();
        assert!(
            excluded,
            "raster arrays must not be duplicated into the plot cache"
        );
        let run:i32=sqlx::query_scalar("INSERT INTO runs (name,integration_params,point_spec) VALUES ('plot-history','{}','{}') RETURNING id").fetch_one(store.pool()).await.unwrap();
        let task:i64=sqlx::query_scalar("INSERT INTO run_tasks (run_id,name,sequence_nr,task,task_toml,state) VALUES ($1,'sample',1,'{\"kind\":\"pause\"}','kind=\"pause\"','active') RETURNING id").bind(run).fetch_one(store.pool()).await.unwrap();
        let state = AccumulatorState::empty_gammaloop();
        let mut raw = state.to_persistent_json().unwrap();
        raw["bundle"] =
            serde_json::json!({"histograms":{"large-unused-payload":"x".repeat(20000)}});
        sqlx::query("INSERT INTO persisted_observable_snapshots (run_id,task_id,persisted_observable) SELECT $1,$2,$3 FROM generate_series(1,600)").bind(run).bind(task).bind(&raw).execute(store.pool()).await.unwrap();
        let compact: Value = sqlx::query_scalar(
            "SELECT plot_observable FROM persisted_observable_snapshots WHERE run_id=$1 LIMIT 1",
        )
        .bind(run)
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert!(compact.to_string().len() * 5 < raw.to_string().len());
        let decoded = AccumulatorState::from_gammaloop_persistent_json(&compact).unwrap();
        assert_eq!(decoded.sample_count(), state.sample_count());
        let (first, last): (i64, i64) = sqlx::query_as(
            "SELECT min(id),max(id) FROM persisted_observable_snapshots WHERE run_id=$1",
        )
        .bind(run)
        .fetch_one(store.pool())
        .await
        .unwrap();
        let page = store.plot_history(run, task, None, 256).await.unwrap();
        assert!(page.reset);
        assert!(page.snapshots.len() <= 256);
        assert_eq!(page.snapshots.first().unwrap().id, last.to_string());
        assert_eq!(page.snapshots.last().unwrap().id, first.to_string());
        assert_eq!(page.head, last);
        assert!(
            store
                .plot_history(run, task, Some(last), 256)
                .await
                .unwrap()
                .snapshots
                .is_empty()
        );
        // Old rows stay readable without a bulk migration/backfill.
        sqlx::query("UPDATE persisted_observable_snapshots SET plot_observable=NULL WHERE id=$1")
            .bind(first)
            .execute(store.pool())
            .await
            .unwrap();
        let legacy = store.plot_history(run, task, None, 256).await.unwrap();
        assert_eq!(legacy.snapshots.last().unwrap().persisted_output, compact);
        sqlx::query("DELETE FROM persisted_observable_snapshots WHERE run_id=$1 AND id>$2")
            .bind(run)
            .bind(first + 10)
            .execute(store.pool())
            .await
            .unwrap();
        let reset = store
            .plot_history(run, task, Some(last), 256)
            .await
            .unwrap();
        assert!(reset.reset);
        assert_eq!(reset.head, first + 10);
        assert_eq!(reset.snapshots.len(), 11);
        let next:i64=sqlx::query_scalar("INSERT INTO persisted_observable_snapshots (run_id,task_id,persisted_observable) VALUES ($1,$2,$3) RETURNING id").bind(run).bind(task).bind(raw).fetch_one(store.pool()).await.unwrap();
        let delta = store
            .plot_history(run, task, Some(first + 10), 256)
            .await
            .unwrap();
        assert!(!delta.reset);
        assert_eq!(delta.snapshots.len(), 1);
        assert_eq!(delta.head, next);
        sqlx::query("DELETE FROM runs WHERE id=$1")
            .bind(run)
            .execute(store.pool())
            .await
            .unwrap();
    }
}
