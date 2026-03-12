use crate::core::{StoreError, WorkerRole};
use tokio::sync::watch;
use tracing::Instrument;
use tracing::warn;

use super::NodeRunnerStore;
use super::evaluator_role_runner::run_evaluator_role;
use super::sampler_aggregator_role_runner::run_sampler_aggregator_role;

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

    pub(super) async fn run(self, stop_rx: watch::Receiver<bool>) -> Result<(), StoreError> {
        let span = tracing::span!(
            tracing::Level::TRACE,
            "worker_role_context",
            source = "worker",
            run_id = self.run_id,
            node_id = %self.node_id,
            worker_id = %self.node_id,
            role = %self.role
        );
        async move {
            match self.role {
                WorkerRole::Evaluator => run_evaluator_role(&self, stop_rx).await,
                WorkerRole::SamplerAggregator => run_sampler_aggregator_role(&self, stop_rx).await,
            }
        }
        .instrument(span)
        .await
    }

    pub(super) async fn mark_active_with_log(&self) -> Result<(), StoreError> {
        self.store
            .set_current_assignment(&self.node_id, self.role, self.run_id)
            .await
    }

    pub(super) async fn heartbeat_with_log(&self) {
        if let Err(err) = self.store.heartbeat_node(&self.node_id).await {
            warn!("node heartbeat failed: {err}");
        }
    }

    pub(super) async fn mark_inactive_with_log(&self) {
        if let Err(err) = self.store.clear_current_assignment(&self.node_id).await {
            warn!("failed to clear current node assignment: {err}");
        }
    }
}
