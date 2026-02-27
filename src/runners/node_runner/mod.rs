//! Node-local worker orchestration and role reconciliation.
//!
//! A `run_node` process is role-agnostic. Desired role/run comes from DB.
//! The supervisor loop polls desired assignment and starts/stops one worker task.

mod active_worker;
mod reconcile;

use crate::core::{
    AggregationStore, AssignmentLeaseStore, ControlPlaneStore, RunInitMetadataStore, RunSpecStore,
    StoreError, WorkQueueStore, WorkerRegistryStore, WorkerRole,
};
use std::time::Duration;
use tokio::{sync::watch, task::JoinHandle, time::sleep};
use tracing::info;

use self::active_worker::ActiveWorker;

#[derive(Debug, Clone)]
pub struct NodeRunnerConfig {
    pub poll_interval: Duration,
}

pub trait NodeRunnerStore:
    RunSpecStore
    + RunInitMetadataStore
    + ControlPlaneStore
    + WorkerRegistryStore
    + AssignmentLeaseStore
    + WorkQueueStore
    + AggregationStore
    + Clone
    + Send
    + Sync
    + 'static
{
}

impl<T> NodeRunnerStore for T where
    T: RunSpecStore
        + RunInitMetadataStore
        + ControlPlaneStore
        + WorkerRegistryStore
        + AssignmentLeaseStore
        + WorkQueueStore
        + AggregationStore
        + Clone
        + Send
        + Sync
        + 'static
{
}

impl Default for NodeRunnerConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(1_000),
        }
    }
}

pub(super) fn role_worker_id(node_id: &str, role: WorkerRole) -> String {
    format!("{node_id}-{role}")
}

pub(super) const ROLE_TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RoleTarget {
    pub(super) role: WorkerRole,
    pub(super) run_id: i32,
}

pub struct NodeRunner<S: NodeRunnerStore> {
    store: S,
    node_id: String,
    config: NodeRunnerConfig,
    role: Option<WorkerRole>,
    run_id: Option<i32>,
    worker_id: Option<String>,
    stop_tx: Option<watch::Sender<bool>>,
    handle: Option<JoinHandle<()>>,
}

impl<S: NodeRunnerStore> NodeRunner<S> {
    pub fn new(store: S, node_id: impl Into<String>, config: NodeRunnerConfig) -> Self {
        Self {
            store,
            node_id: node_id.into(),
            config,
            role: None,
            run_id: None,
            worker_id: None,
            stop_tx: None,
            handle: None,
        }
    }

    fn current_target(&self) -> Option<RoleTarget> {
        match (self.role, self.run_id) {
            (Some(role), Some(run_id)) => Some(RoleTarget { role, run_id }),
            _ => None,
        }
    }

    pub async fn run(mut self) -> Result<(), StoreError> {
        let mut shutdown = std::pin::pin!(tokio::signal::ctrl_c());

        loop {
            if self
                .store
                .consume_node_shutdown_request(&self.node_id)
                .await?
            {
                info!(node_id = %self.node_id, "node shutdown requested by control-plane");
                break;
            }

            let desired_target = self.resolve_desired_target().await?;
            self.reconcile(desired_target).await?;

            tokio::select! {
                _ = &mut shutdown => {
                    info!(node_id = %self.node_id, "stopping node-runner");
                    break;
                }
                _ = sleep(self.config.poll_interval) => {}
            }
        }

        self.stop_current().await;
        Ok(())
    }
}

pub(super) fn binary_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
