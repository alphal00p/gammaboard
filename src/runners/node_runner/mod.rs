//! Node-local worker orchestration and role reconciliation.
//!
//! A `run-node` process is role-agnostic. Desired role/run comes from DB.
//! The supervisor loop polls desired assignment, reconciles one in-process role runner,
//! and ticks it until desired assignment changes or the role finishes.

mod active_worker;
mod reconcile;
mod role_runner;

use crate::core::{
    ControlPlaneStore, EvaluatorWorkerStore, RunSpecStore, SamplerWorkerStore, StoreError,
    WorkerRole,
};
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::{Instrument, info, warn};
use uuid::Uuid;

use self::active_worker::ActiveWorker;
use self::role_runner::RoleRunner;

#[derive(Debug, Clone)]
pub struct NodeRunnerConfig {
    pub min_tick_time: Duration,
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
            min_tick_time: Duration::from_millis(1_000),
            max_consecutive_start_failures: 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RoleTarget {
    pub(super) role: WorkerRole,
    pub(super) run_id: i32,
}

pub(super) struct ActiveRoleRunner<S: NodeRunnerStore> {
    target: RoleTarget,
    worker: ActiveWorker<S>,
    context_span: tracing::Span,
    runner: Box<dyn RoleRunner>,
}

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct RetryState {
    pub(super) blocked_target: Option<RoleTarget>,
    pub(super) failure_target: Option<RoleTarget>,
    pub(super) consecutive_failures: u32,
}

impl RetryState {
    fn reset_for_desired_target_change(&mut self, desired_target: Option<RoleTarget>) {
        if desired_target != self.failure_target {
            self.failure_target = desired_target;
            self.consecutive_failures = 0;
            if self.blocked_target != desired_target {
                self.blocked_target = None;
            }
        }
    }

    fn is_blocked(&self, target: RoleTarget) -> bool {
        self.blocked_target == Some(target)
    }

    fn clear(&mut self) {
        self.failure_target = None;
        self.consecutive_failures = 0;
        self.blocked_target = None;
    }

    fn note_failure(&mut self, target: RoleTarget, max_consecutive_failures: u32) -> bool {
        if self.failure_target == Some(target) {
            self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        } else {
            self.failure_target = Some(target);
            self.consecutive_failures = 1;
        }
        if self.consecutive_failures >= max_consecutive_failures {
            self.blocked_target = Some(target);
            return true;
        }
        false
    }
}

pub struct NodeRunner<S: NodeRunnerStore> {
    store: S,
    node_name: String,
    node_uuid: String,
    config: NodeRunnerConfig,
    active_runner: Option<ActiveRoleRunner<S>>,
    retry_state: RetryState,
}

impl<S: NodeRunnerStore> NodeRunner<S> {
    pub fn new(store: S, node_name: impl Into<String>, config: NodeRunnerConfig) -> Self {
        Self {
            store,
            node_name: node_name.into(),
            node_uuid: Uuid::new_v4().to_string(),
            config,
            active_runner: None,
            retry_state: RetryState::default(),
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
            node_name = %self.node_name,
            node_uuid = %self.node_uuid
        );
        async move {
            let mut shutdown = std::pin::pin!(tokio::signal::ctrl_c());
            let mut announce_failed_at: Option<Instant> = None;

            loop {
                let tick_started = Instant::now();
                if let Err(err) = self
                    .store
                    .announce_node(&self.node_name, &self.node_uuid)
                    .await
                {
                    warn!("node announce failed: {err}");
                    let failed_at = *announce_failed_at.get_or_insert_with(Instant::now);
                    if failed_at.elapsed() >= Duration::from_secs(30) {
                        warn!("node announce failed for 30 seconds; shutting down node-runner");
                        break;
                    }
                    tokio::select! {
                        _ = &mut shutdown => {
                            info!("stopping node-runner");
                            break;
                        }
                        _ = sleep(self.config.min_tick_time) => {}
                    }
                    continue;
                }
                announce_failed_at = None;

                if self
                    .store
                    .consume_node_shutdown_request(&self.node_uuid)
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
                            self.fail_current_assignment(target, &err).await?;
                            false
                        }
                    };
                    if done {
                        self.finish_current_assignment().await?;
                    }
                    let elapsed = tick_started.elapsed();
                    if elapsed < self.config.min_tick_time {
                        tokio::select! {
                            _ = &mut shutdown => {
                                info!("stopping node-runner");
                                break;
                            }
                            _ = sleep(self.config.min_tick_time - elapsed) => {}
                        }
                        continue;
                    }
                    continue;
                }

                tokio::select! {
                    _ = &mut shutdown => {
                        info!("stopping node-runner");
                        break;
                    }
                    _ = sleep(self.config.min_tick_time) => {}
                }
            }

            self.stop_current().await;
            if let Err(err) = self.store.expire_node_lease(&self.node_uuid).await {
                warn!("failed to expire node lease on shutdown: {err}");
            }
            Ok(())
        }
        .instrument(span)
        .await
    }
}
