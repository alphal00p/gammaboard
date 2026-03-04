use crate::core::{StoreError, Worker as WorkerRecord, WorkerRole, WorkerStatus};
use serde_json::json;
use tokio::sync::watch;
use tracing::Instrument;
use tracing::warn;

use super::evaluator_role_runner::run_evaluator_role;
use super::sampler_aggregator_role_runner::run_sampler_aggregator_role;
use super::{NodeRunnerStore, binary_version};

pub(super) struct ActiveWorker<S: NodeRunnerStore> {
    pub(super) store: S,
    pub(super) node_id: String,
    pub(super) worker_id: String,
    pub(super) role: WorkerRole,
    pub(super) run_id: i32,
}

impl<S: NodeRunnerStore> ActiveWorker<S> {
    pub(super) fn new(
        store: S,
        node_id: impl Into<String>,
        worker_id: impl Into<String>,
        role: WorkerRole,
        run_id: i32,
    ) -> Self {
        Self {
            store,
            node_id: node_id.into(),
            worker_id: worker_id.into(),
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
            worker_id = %self.worker_id
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

    pub(super) async fn register_active_worker(
        &self,
        implementation: &str,
    ) -> Result<(), StoreError> {
        self.store
            .register_worker(&WorkerRecord {
                worker_id: self.worker_id.clone(),
                node_id: Some(self.node_id.clone()),
                role: self.role,
                implementation: implementation.to_string(),
                version: binary_version().to_string(),
                node_specs: json!({ "node_id": self.node_id }),
                status: WorkerStatus::Active,
                last_seen: None,
            })
            .await
    }

    pub(super) async fn heartbeat_with_log(&self) {
        if let Err(err) = self.store.heartbeat_worker(&self.worker_id).await {
            warn!("worker heartbeat failed: {err}");
        }
    }

    pub(super) async fn mark_inactive_with_log(&self) {
        if let Err(err) = self
            .store
            .update_worker_status(&self.worker_id, WorkerStatus::Inactive)
            .await
        {
            warn!("failed to mark worker inactive: {err}");
        }
    }
}
