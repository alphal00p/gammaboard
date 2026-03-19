//! Node-local worker orchestration and role reconciliation.
//!
//! A `run-node` process is role-agnostic. Desired role/run comes from DB.
//! The supervisor loop polls desired assignment, reconciles one in-process role runner,
//! and ticks it until desired assignment changes or the role finishes.

mod active_worker;
mod evaluator_role_runner;
mod reconcile;
mod sampler_aggregator_role_runner;

use crate::core::{
    ControlPlaneStore, EvaluatorWorkerStore, RunSpecStore, SamplerWorkerStore, StoreError,
    WorkerRole,
};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{Instrument, info, warn};

use self::active_worker::ActiveWorker;
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct NodeRunnerConfig {
    pub poll_interval: Duration,
    pub max_consecutive_start_failures: u32,
}

pub trait NodeRunnerStore:
    RunSpecStore
    + ControlPlaneStore
    + EvaluatorWorkerStore
    + SamplerWorkerStore
    + Clone
    + Send
    + Sync
    + 'static
{
}

impl<T> NodeRunnerStore for T where
    T: RunSpecStore
        + ControlPlaneStore
        + EvaluatorWorkerStore
        + SamplerWorkerStore
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RoleTarget {
    pub(super) role: WorkerRole,
    pub(super) run_id: i32,
}

#[async_trait(?Send)]
pub(super) trait RoleRunner {
    async fn tick(&mut self) -> Result<bool, StoreError>;
    async fn persist_state(&mut self) -> Result<(), StoreError>;
}

pub(super) struct ActiveRoleRunner<S: NodeRunnerStore> {
    target: RoleTarget,
    worker: ActiveWorker<S>,
    context_span: tracing::Span,
    runner: Box<dyn RoleRunner>,
}

pub struct NodeRunner<S: NodeRunnerStore> {
    store: S,
    node_id: String,
    config: NodeRunnerConfig,
    active_runner: Option<ActiveRoleRunner<S>>,
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
            active_runner: None,
            blocked_target: None,
            failure_target: None,
            consecutive_start_failures: 0,
        }
    }

    fn current_target(&self) -> Option<RoleTarget> {
        self.active_runner.as_ref().map(|runner| runner.target)
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

                if self.active_runner.is_some() {
                    let tick_outcome = {
                        let active_runner = self.active_runner.as_mut().expect("checked above");
                        let target = active_runner.target;
                        let result = active_runner
                            .runner
                            .tick()
                            .instrument(active_runner.context_span.clone())
                            .await;
                        (target, result)
                    };
                    let (target, result) = tick_outcome;
                    let done = match result {
                        Ok(done) => done,
                        Err(err) => {
                            warn!("role runner tick failed: {err}");
                            self.note_start_failure(target);
                            self.stop_current().await;
                            sleep(self.config.poll_interval).await;
                            continue;
                        }
                    };
                    if done {
                        self.note_role_stopped();
                        self.stop_current().await;
                        continue;
                    }
                    continue;
                }

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
