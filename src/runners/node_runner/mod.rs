//! Node-local worker orchestration and role reconciliation.
//!
//! A `run-node` process is role-agnostic. Desired role/run comes from DB.
//! The supervisor loop polls desired assignment and starts/stops one worker task.

mod active_worker;
mod evaluator_role_runner;
mod reconcile;
mod sampler_aggregator_role_runner;

use crate::core::{
    AggregationStore, ControlPlaneStore, RunSpecStore, StoreError, WorkQueueStore, WorkerRole,
};
use std::time::Duration;
use tokio::{sync::watch, task::JoinHandle, time::sleep};
use tracing::{Instrument, info};

use self::active_worker::ActiveWorker;

#[derive(Debug, Clone)]
pub struct NodeRunnerConfig {
    pub poll_interval: Duration,
    pub max_consecutive_start_failures: u32,
}

pub trait NodeRunnerStore:
    RunSpecStore + ControlPlaneStore + WorkQueueStore + AggregationStore + Clone + Send + Sync + 'static
{
}

impl<T> NodeRunnerStore for T where
    T: RunSpecStore
        + ControlPlaneStore
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
            max_consecutive_start_failures: 3,
        }
    }
}

pub(super) const ROLE_TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RoleTarget {
    pub(super) role: WorkerRole,
    pub(super) run_id: i32,
}

pub(super) struct ActiveRoleTask {
    pub(super) target: RoleTarget,
    pub(super) context_span: tracing::Span,
    pub(super) stop_tx: watch::Sender<bool>,
    pub(super) handle: JoinHandle<Result<(), StoreError>>,
}

pub struct NodeRunner<S: NodeRunnerStore> {
    store: S,
    node_id: String,
    config: NodeRunnerConfig,
    active_task: Option<ActiveRoleTask>,
    blocked_target: Option<RoleTarget>,
    failure_target: Option<RoleTarget>,
    consecutive_start_failures: u32,
}

impl<S: NodeRunnerStore> NodeRunner<S> {
    pub fn new(store: S, node_id: impl Into<String>, config: NodeRunnerConfig) -> Self {
        Self {
            store,
            node_id: node_id.into(),
            config,
            active_task: None,
            blocked_target: None,
            failure_target: None,
            consecutive_start_failures: 0,
        }
    }

    fn current_target(&self) -> Option<RoleTarget> {
        self.active_task.as_ref().map(|task| task.target)
    }

    pub async fn run(mut self) -> Result<(), StoreError> {
        let span = tracing::span!(
            tracing::Level::TRACE,
            "node_runner_context",
            source = "worker",
            node_id = %self.node_id
        );
        async move {
            let mut shutdown = std::pin::pin!(tokio::signal::ctrl_c());
            self.store.register_node(&self.node_id).await?;

            loop {
                self.store.heartbeat_node(&self.node_id).await?;
                if self
                    .store
                    .consume_node_shutdown_request(&self.node_id)
                    .await?
                {
                    info!("node shutdown requested by control-plane");
                    break;
                }

                let desired_target = self.resolve_desired_target().await?;
                self.reconcile(desired_target).await?;

                tokio::select! {
                    _ = &mut shutdown => {
                        info!("stopping node-runner");
                        break;
                    }
                    _ = sleep(self.config.poll_interval) => {}
                }
            }

            self.stop_current().await;
            Ok(())
        }
        .instrument(span)
        .await
    }
}

pub(super) fn binary_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
