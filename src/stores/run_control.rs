use crate::core::{ControlPlaneStore, RunStatus, StoreError};
use async_trait::async_trait;

#[async_trait]
pub trait RunControlStore: Send + Sync {
    async fn stop_run_and_clear_assignments(&self, run_id: i32) -> Result<u64, StoreError>;
}

#[async_trait]
impl<T> RunControlStore for T
where
    T: ControlPlaneStore + Send + Sync,
{
    async fn stop_run_and_clear_assignments(&self, run_id: i32) -> Result<u64, StoreError> {
        self.set_run_status(run_id, RunStatus::Cancelled).await?;
        self.clear_desired_assignments_for_run(run_id).await
    }
}
