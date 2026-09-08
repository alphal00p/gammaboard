use super::{PgStore, queries};
use crate::core::{RunReadStore, StoreError};

#[async_trait::async_trait]
impl RunReadStore for PgStore {
    async fn health_check(&self) -> Result<(), StoreError> {
        queries::health_check(&self.pool).await?;
        Ok(())
    }

    async fn get_all_runs(&self) -> Result<Vec<crate::stores::RunProgress>, StoreError> {
        Ok(queries::get_all_runs(&self.pool).await?)
    }

    async fn get_run_progress(
        &self,
        run_id: i32,
    ) -> Result<Option<crate::stores::RunProgress>, StoreError> {
        Ok(queries::get_run_progress(&self.pool, run_id).await?)
    }

    async fn get_runs_by_name(
        &self,
        run_name: &str,
    ) -> Result<Vec<crate::stores::RunProgress>, StoreError> {
        Ok(queries::get_runs_by_name(&self.pool, run_name).await?)
    }

    async fn get_runs_page(
        &self,
        limit: usize,
        offset: usize,
        include_children: bool,
    ) -> Result<Vec<crate::stores::RunProgress>, StoreError> {
        Ok(queries::get_runs_page(&self.pool, limit, offset, include_children).await?)
    }

    async fn get_control_plane_run_ids(&self) -> Result<Vec<i32>, StoreError> {
        Ok(queries::get_control_plane_run_ids(&self.pool).await?)
    }

    async fn get_child_runs_for_task(
        &self,
        parent_run_id: i32,
        parent_task_id: i64,
        spawn_kind: &str,
    ) -> Result<Vec<crate::stores::RunProgress>, StoreError> {
        Ok(
            queries::get_child_runs_for_task(&self.pool, parent_run_id, parent_task_id, spawn_kind)
                .await?,
        )
    }

    async fn get_task_output_snapshots(
        &self,
        run_id: i32,
        task_id: i64,
        after_snapshot_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<crate::stores::TaskOutputSnapshot>, StoreError> {
        Ok(queries::get_task_output_snapshots(
            &self.pool,
            run_id,
            task_id,
            after_snapshot_id,
            limit,
        )
        .await?)
    }

    async fn get_latest_task_stage_snapshot(
        &self,
        run_id: i32,
        task_id: i64,
    ) -> Result<Option<crate::stores::TaskStageSnapshot>, StoreError> {
        Ok(queries::get_latest_task_stage_snapshot(&self.pool, run_id, task_id).await?)
    }

    async fn get_latest_task_stage_snapshot_id(
        &self,
        run_id: i32,
        task_id: i64,
    ) -> Result<Option<String>, StoreError> {
        Ok(queries::get_latest_task_stage_snapshot_id(&self.pool, run_id, task_id).await?)
    }

    async fn get_runtime_logs(
        &self,
        limit: i64,
        source: Option<&str>,
        run_id: Option<i32>,
        include_child_runs: bool,
        node_name: Option<&str>,
        node_uuid: Option<&str>,
        level: Option<&str>,
        query: Option<&str>,
        before_id: Option<i64>,
    ) -> Result<crate::stores::RuntimeLogPage, StoreError> {
        Ok(queries::get_runtime_logs(
            &self.pool,
            limit,
            source,
            run_id,
            include_child_runs,
            node_name,
            node_uuid,
            level,
            query,
            before_id,
        )
        .await?)
    }

    async fn get_registered_workers(
        &self,
        run_id: Option<i32>,
    ) -> Result<Vec<crate::stores::RegisteredWorkerEntry>, StoreError> {
        Ok(queries::get_registered_workers(&self.pool, run_id).await?)
    }

    async fn get_evaluator_performance_history(
        &self,
        run_id: i32,
        limit: i64,
        worker_id: Option<&str>,
    ) -> Result<Vec<crate::stores::EvaluatorPerformanceHistoryEntry>, StoreError> {
        Ok(
            queries::get_evaluator_performance_history(&self.pool, run_id, limit, worker_id)
                .await?,
        )
    }

    async fn get_sampler_performance_history(
        &self,
        run_id: i32,
        limit: i64,
        worker_id: Option<&str>,
    ) -> Result<Vec<crate::stores::SamplerPerformanceHistoryEntry>, StoreError> {
        Ok(queries::get_sampler_performance_history(&self.pool, run_id, limit, worker_id).await?)
    }
}
