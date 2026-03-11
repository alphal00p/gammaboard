use crate::core::{ControlPlaneStore, StoreError};
use async_trait::async_trait;

#[async_trait]
pub trait RunControlStore: Send + Sync {
    async fn clear_run_assignments(&self, run_id: i32) -> Result<u64, StoreError>;
}

#[async_trait]
impl<T> RunControlStore for T
where
    T: ControlPlaneStore + Send + Sync,
{
    async fn clear_run_assignments(&self, run_id: i32) -> Result<u64, StoreError> {
        self.clear_desired_assignments_for_run(run_id).await
    }
}
