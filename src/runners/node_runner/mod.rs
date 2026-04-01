//! Node-local worker orchestration and role reconciliation.
//!
//! A `node run` process is role-agnostic. Desired role/run comes from DB.
//! The supervisor loop polls desired assignment, reconciles one in-process role runner,
//! and ticks it until desired assignment changes or the role finishes.

mod active_worker;
mod reconcile;
mod role_runner;

use crate::core::{
    ControlPlaneStore, EvaluatorWorkerStore, RunSpecStore, SamplerWorkerStore, StoreError,
    WorkerRole,
};
use rand::Rng;
use std::time::{Duration, Instant};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
    time::sleep,
};
use tracing::{Instrument, info, warn};
use uuid::Uuid;

use self::active_worker::ActiveWorker;
use self::role_runner::RoleRunner;

#[derive(Debug, Clone)]
pub struct NodeRunnerConfig {
    pub min_tick_time: Duration,
    pub max_consecutive_start_failures: u32,
    pub reconcile_initial_backoff: Duration,
    pub reconcile_backoff_factor: f64,
    pub reconcile_max_backoff: Duration,
    pub announce_interval: Duration,
    pub announce_retry_interval: Duration,
    pub announce_failure_timeout: Duration,
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
            min_tick_time: Duration::from_millis(50),
            max_consecutive_start_failures: 3,
            reconcile_initial_backoff: Duration::from_millis(50),
            reconcile_backoff_factor: 2.0,
            reconcile_max_backoff: Duration::from_millis(2_000),
            announce_interval: Duration::from_secs(2),
            announce_retry_interval: Duration::from_millis(500),
            announce_failure_timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Debug)]
enum LeaseEvent {
    Ready,
    Fatal {
        startup: bool,
        retries: u32,
        last_error: String,
    },
}

struct LeaseRenewalHandle {
    shutdown: watch::Sender<bool>,
    events: mpsc::UnboundedReceiver<LeaseEvent>,
    join_handle: JoinHandle<()>,
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
    reconcile_backoff: Duration,
}

impl<S: NodeRunnerStore> NodeRunner<S> {
    pub fn new(store: S, node_name: impl Into<String>, config: NodeRunnerConfig) -> Self {
        Self {
            store,
            node_name: node_name.into(),
            node_uuid: Uuid::new_v4().to_string(),
            reconcile_backoff: config.reconcile_initial_backoff,
            config,
            active_runner: None,
            retry_state: RetryState::default(),
        }
    }

    pub(super) fn reset_reconcile_backoff(&mut self) {
        self.reconcile_backoff = self.config.reconcile_initial_backoff;
    }

    fn next_reconcile_sleep(&mut self) -> Duration {
        let current = self.reconcile_backoff;
        let next_secs = (current.as_secs_f64() * self.config.reconcile_backoff_factor)
            .min(self.config.reconcile_max_backoff.as_secs_f64());
        self.reconcile_backoff = Duration::from_secs_f64(next_secs);
        let mut rng = rand::rng();
        let jitter = rng.random_range(0.5..=1.5);
        Duration::from_secs_f64((current.as_secs_f64() * jitter).max(0.0))
    }

    fn current_target(&self) -> Option<RoleTarget> {
        self.active_runner.as_ref().map(|runner| runner.target)
    }

    fn spawn_lease_renewal_task(&self) -> LeaseRenewalHandle {
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let store = self.store.clone();
        let node_name = self.node_name.clone();
        let node_uuid = self.node_uuid.clone();
        let announce_interval = self.config.announce_interval;
        let announce_retry_interval = self.config.announce_retry_interval;
        let announce_failure_timeout = self.config.announce_failure_timeout;
        let join_handle = tokio::spawn(async move {
            let mut startup = true;
            let mut announce_failures = 0u32;
            let mut announce_failed_at: Option<Instant> = None;
            loop {
                match store.announce_node(&node_name, &node_uuid).await {
                    Ok(()) => {
                        if announce_failures > 0 {
                            info!(
                                startup,
                                retries = announce_failures,
                                downtime_ms = announce_failed_at
                                    .map(|failed_at| failed_at.elapsed().as_millis() as u64)
                                    .unwrap_or(0),
                                "node announce recovered after retries"
                            );
                        }
                        if startup {
                            let _ = event_tx.send(LeaseEvent::Ready);
                            info!("node startup announce succeeded");
                            startup = false;
                        }
                        announce_failures = 0;
                        announce_failed_at = None;
                        tokio::select! {
                            _ = shutdown_rx.changed() => {
                                if *shutdown_rx.borrow() {
                                    break;
                                }
                            }
                            _ = sleep(announce_interval) => {}
                        }
                    }
                    Err(err) => {
                        announce_failures = announce_failures.saturating_add(1);
                        let failed_at = *announce_failed_at.get_or_insert_with(Instant::now);
                        let elapsed = failed_at.elapsed();
                        let should_log = announce_failures == 1 || announce_failures % 10 == 0;
                        if should_log {
                            warn!(
                                startup,
                                retries = announce_failures,
                                elapsed_ms = elapsed.as_millis() as u64,
                                last_error = %err,
                                "node announce failed; retrying"
                            );
                        }
                        if elapsed >= announce_failure_timeout {
                            let _ = event_tx.send(LeaseEvent::Fatal {
                                startup,
                                retries: announce_failures,
                                last_error: err.to_string(),
                            });
                            return;
                        }
                        tokio::select! {
                            _ = shutdown_rx.changed() => {
                                if *shutdown_rx.borrow() {
                                    break;
                                }
                            }
                            _ = sleep(announce_retry_interval) => {}
                        }
                    }
                }
            }
        });

        LeaseRenewalHandle {
            shutdown: shutdown_tx,
            events: event_rx,
            join_handle,
        }
    }

    async fn stop_lease_renewal_task(handle: LeaseRenewalHandle) {
        let LeaseRenewalHandle {
            shutdown,
            join_handle,
            ..
        } = handle;
        let _ = shutdown.send(true);
        if let Err(err) = join_handle.await {
            warn!("lease renewal task failed to join cleanly: {err}");
        }
    }

    async fn wait_for_initial_lease(
        lease_events: &mut mpsc::UnboundedReceiver<LeaseEvent>,
        shutdown: &mut std::pin::Pin<
            &mut impl std::future::Future<Output = Result<(), std::io::Error>>,
        >,
        #[cfg(unix)] sigterm: &mut tokio::signal::unix::Signal,
    ) -> Result<bool, StoreError> {
        loop {
            #[cfg(unix)]
            tokio::select! {
                _ = shutdown.as_mut() => {
                    info!("stopping node-runner");
                    return Ok(false);
                }
                _ = sigterm.recv() => {
                    info!("stopping node-runner (SIGTERM)");
                    return Ok(false);
                }
                event = lease_events.recv() => {
                    match event {
                        Some(LeaseEvent::Ready) => return Ok(true),
                        Some(LeaseEvent::Fatal { startup, retries, last_error }) => {
                            warn!(
                                startup,
                                retries,
                                last_error = %last_error,
                                "node announce failed for too long; shutting down node-runner"
                            );
                            return Ok(false);
                        }
                        None => return Err(StoreError::store("lease renewal task exited before startup completed")),
                    }
                }
            }
            #[cfg(not(unix))]
            tokio::select! {
                _ = shutdown.as_mut() => {
                    info!("stopping node-runner");
                    return Ok(false);
                }
                event = lease_events.recv() => {
                    match event {
                        Some(LeaseEvent::Ready) => return Ok(true),
                        Some(LeaseEvent::Fatal { startup, retries, last_error }) => {
                            warn!(
                                startup,
                                retries,
                                last_error = %last_error,
                                "node announce failed for too long; shutting down node-runner"
                            );
                            return Ok(false);
                        }
                        None => return Err(StoreError::store("lease renewal task exited before startup completed")),
                    }
                }
            }
        }
    }

    async fn poll_lease_failure(
        lease_events: &mut mpsc::UnboundedReceiver<LeaseEvent>,
    ) -> Result<bool, StoreError> {
        match lease_events.try_recv() {
            Ok(LeaseEvent::Ready) => Ok(false),
            Ok(LeaseEvent::Fatal {
                startup,
                retries,
                last_error,
            }) => {
                warn!(
                    startup,
                    retries,
                    last_error = %last_error,
                    "node announce failed for too long; shutting down node-runner"
                );
                Ok(true)
            }
            Err(mpsc::error::TryRecvError::Empty) => Ok(false),
            Err(mpsc::error::TryRecvError::Disconnected) => {
                Err(StoreError::store("lease renewal task exited unexpectedly"))
            }
        }
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
            #[cfg(unix)]
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).map_err(
                    |err| StoreError::store(format!("failed to install SIGTERM handler: {err}")),
                )?;
            let mut lease_renewal = self.spawn_lease_renewal_task();

            #[cfg(unix)]
            let startup_announced = Self::wait_for_initial_lease(
                &mut lease_renewal.events,
                &mut shutdown,
                &mut sigterm,
            )
            .await?;
            #[cfg(not(unix))]
            let startup_announced =
                Self::wait_for_initial_lease(&mut lease_renewal.events, &mut shutdown).await?;

            if !startup_announced {
                Self::stop_lease_renewal_task(lease_renewal).await;
                if let Err(err) = self.store.expire_node_lease(&self.node_uuid).await {
                    warn!("failed to expire node lease on shutdown: {err}");
                }
                return Ok(());
            }

            loop {
                let tick_started = Instant::now();
                if Self::poll_lease_failure(&mut lease_renewal.events).await? {
                    break;
                }

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
                            self.reset_reconcile_backoff();
                            false
                        }
                    };
                    if done {
                        self.finish_current_assignment().await?;
                        self.reset_reconcile_backoff();
                        continue;
                    }
                    if self.active_runner.is_none() {
                        self.reset_reconcile_backoff();
                        continue;
                    }
                    let elapsed = tick_started.elapsed();
                    if elapsed < self.config.min_tick_time {
                        #[cfg(unix)]
                        tokio::select! {
                            _ = &mut shutdown => {
                                info!("stopping node-runner");
                                break;
                            }
                            _ = sigterm.recv() => {
                                info!("stopping node-runner (SIGTERM)");
                                break;
                            }
                            _ = sleep(self.config.min_tick_time - elapsed) => {}
                        }
                        #[cfg(not(unix))]
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

                #[cfg(unix)]
                tokio::select! {
                    _ = &mut shutdown => {
                        info!("stopping node-runner");
                        break;
                    }
                    _ = sigterm.recv() => {
                        info!("stopping node-runner (SIGTERM)");
                        break;
                    }
                    _ = sleep(self.next_reconcile_sleep()) => {}
                }
                #[cfg(not(unix))]
                tokio::select! {
                    _ = &mut shutdown => {
                        info!("stopping node-runner");
                        break;
                    }
                    _ = sleep(self.next_reconcile_sleep()) => {}
                }
            }

            self.stop_current().await;
            Self::stop_lease_renewal_task(lease_renewal).await;
            if let Err(err) = self.store.expire_node_lease(&self.node_uuid).await {
                warn!("failed to expire node lease on shutdown: {err}");
            }
            Ok(())
        }
        .instrument(span)
        .await
    }
}
