use crate::core::{StoreError, WorkerRole};
use tracing::warn;

use super::NodeRunnerStore;

#[derive(Clone)]
pub(super) struct ActiveWorker<S: NodeRunnerStore> {
    pub(super) store: S,
    pub(super) node_id: String,
    pub(super) role: WorkerRole,
    pub(super) run_id: i32,
}

impl<S: NodeRunnerStore> ActiveWorker<S> {
    pub(super) fn new(store: S, node_id: impl Into<String>, role: WorkerRole, run_id: i32) -> Self {
        Self {
            store,
            node_id: node_id.into(),
            role,
            run_id,
        }
    }

    pub(super) async fn mark_active_with_log(&self) -> Result<(), StoreError> {
        self.store
            .set_current_assignment(&self.node_id, self.role, self.run_id)
            .await
    }

    pub(super) async fn mark_inactive_with_log(&self) {
        if let Err(err) = self.store.clear_current_assignment(&self.node_id).await {
            warn!("failed to clear current node assignment: {err}");
        }
    }
}
