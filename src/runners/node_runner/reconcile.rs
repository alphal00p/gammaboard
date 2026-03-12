use super::{
    ActiveRoleTask, ActiveWorker, NodeRunner, NodeRunnerStore, ROLE_TASK_SHUTDOWN_TIMEOUT,
    RoleTarget,
};
use crate::core::StoreError;
use tokio::{task::JoinHandle, time::timeout};
use tracing::{Instrument, error, info, warn};

impl<S: NodeRunnerStore> NodeRunner<S> {
    pub(super) async fn resolve_desired_target(&self) -> Result<Option<RoleTarget>, StoreError> {
        let assignment = self.store.get_desired_assignment(&self.node_id).await?;
        Ok(assignment.map(|assignment| RoleTarget {
            role: assignment.role,
            run_id: assignment.run_id,
        }))
    }

    pub(super) async fn reconcile(
        &mut self,
        desired_target: Option<RoleTarget>,
    ) -> Result<(), StoreError> {
        if desired_target != self.failure_target {
            self.failure_target = desired_target;
            self.consecutive_start_failures = 0;
            if self.blocked_target != desired_target {
                self.blocked_target = None;
            }
        }

        self.reap_finished_task().await;

        if self.current_target() == desired_target {
            return Ok(());
        }

        self.stop_current().await;

        if let Some(target) = desired_target {
            if self.blocked_target == Some(target) {
                return Ok(());
            }
            self.start(target);
        }

        Ok(())
    }

    fn start(&mut self, target: RoleTarget) {
        let role_context_span = tracing::span!(
            tracing::Level::TRACE,
            "role_task_context",
            run_id = target.run_id,
            node_id = %self.node_id,
            role = %target.role
        );
        let role_scope_span = role_context_span.clone();
        let _role_scope = role_scope_span.enter();

        info!("starting role task");

        let runtime = ActiveWorker::new(
            self.store.clone(),
            self.node_id.clone(),
            target.role,
            target.run_id,
        );

        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let role_span_for_task = role_context_span.clone();
        let handle =
            tokio::spawn(async move { runtime.run(stop_rx).await }.instrument(role_span_for_task));

        self.active_task = Some(ActiveRoleTask {
            target,
            context_span: role_context_span,
            stop_tx,
            handle,
        });
    }

    async fn reap_finished_task(&mut self) {
        let Some(task) = self.active_task.as_ref() else {
            return;
        };
        if !task.handle.is_finished() {
            return;
        }

        let Some(task) = self.active_task.take() else {
            return;
        };
        let _role_scope = task.context_span.enter();

        match task.handle.await {
            Ok(Ok(())) => {
                self.failure_target = None;
                self.consecutive_start_failures = 0;
                self.blocked_target = None;
            }
            Ok(Err(err)) => {
                error!("role task failed: {err}");
                if self.failure_target == Some(task.target) {
                    self.consecutive_start_failures =
                        self.consecutive_start_failures.saturating_add(1);
                } else {
                    self.failure_target = Some(task.target);
                    self.consecutive_start_failures = 1;
                }
                if self.consecutive_start_failures >= self.config.max_consecutive_start_failures {
                    self.blocked_target = Some(task.target);
                    warn!(
                        role = %task.target.role,
                        run_id = task.target.run_id,
                        consecutive_failures = self.consecutive_start_failures,
                        max_consecutive_start_failures = self.config.max_consecutive_start_failures,
                        "aborting role task restarts after repeated startup failures; waiting for desired assignment change"
                    );
                }
            }
            Err(err) => {
                warn!("role task join failed: {err}");
            }
        }

        warn!("role task exited; waiting for supervisor reconcile");
    }

    pub(super) async fn stop_current(&mut self) {
        let Some(task) = self.active_task.take() else {
            return;
        };
        let _role_scope = task.context_span.enter();

        info!("stopping role task");

        let _ = task.stop_tx.send(true);

        let mut handle: JoinHandle<Result<(), StoreError>> = task.handle;
        match timeout(ROLE_TASK_SHUTDOWN_TIMEOUT, &mut handle).await {
            Ok(join_result) => {
                if let Err(err) = join_result {
                    warn!("role task join failed: {err}");
                }
            }
            Err(_) => {
                warn!("timed out waiting for role task shutdown; aborting task");
                handle.abort();
            }
        }
    }
}
