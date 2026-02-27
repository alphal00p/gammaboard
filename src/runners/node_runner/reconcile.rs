use super::{
    ActiveRoleTask, ActiveWorker, NodeRunner, NodeRunnerStore, ROLE_TASK_SHUTDOWN_TIMEOUT,
    RoleTarget,
};
use crate::core::StoreError;
use tokio::{task::JoinHandle, time::timeout};
use tracing::{error, info, warn};

impl<S: NodeRunnerStore> NodeRunner<S> {
    pub(super) async fn resolve_desired_target(&self) -> Result<Option<RoleTarget>, StoreError> {
        let assignments = self
            .store
            .list_desired_assignments(Some(&self.node_id))
            .await?;

        if assignments.is_empty() {
            return Ok(None);
        }

        if assignments.len() == 1 {
            let assignment = &assignments[0];
            return Ok(Some(RoleTarget {
                role: assignment.role,
                run_id: assignment.run_id,
            }));
        }

        if let Some(current) = self.current_target()
            && let Some(matching) = assignments
                .iter()
                .find(|assignment| assignment.role == current.role)
        {
            warn!(
                node_id = %self.node_id,
                current_role = %current.role,
                conflict_count = assignments.len(),
                "multiple desired role assignments for one node; keeping current role assignment"
            );
            return Ok(Some(RoleTarget {
                role: matching.role,
                run_id: matching.run_id,
            }));
        }

        warn!(
            node_id = %self.node_id,
            conflict_count = assignments.len(),
            "multiple desired role assignments for one node; no active role selected"
        );
        Ok(None)
    }

    pub(super) async fn reconcile(
        &mut self,
        desired_target: Option<RoleTarget>,
    ) -> Result<(), StoreError> {
        self.reap_finished_task().await;

        if self.current_target() == desired_target {
            return Ok(());
        }

        self.stop_current().await;

        if let Some(target) = desired_target {
            self.start(target);
        }

        Ok(())
    }

    fn start(&mut self, target: RoleTarget) {
        let worker_id = super::role_worker_id(&self.node_id, target.role);

        info!(
            target: "worker_log",
            run_id = target.run_id,
            node_id = %self.node_id,
            worker_id = %worker_id,
            role = %target.role,
            event_type = "role_start",
            "starting role task"
        );

        let runtime = ActiveWorker::new(
            self.store.clone(),
            self.node_id.clone(),
            worker_id.clone(),
            target.role,
            target.run_id,
        );

        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let worker_id_for_task = worker_id.clone();
        let handle = tokio::spawn(async move {
            let result = runtime.run(stop_rx).await;
            if let Err(err) = result {
                error!(
                    target: "worker_log",
                    run_id = target.run_id,
                    worker_id = %worker_id_for_task,
                    role = %target.role,
                    event_type = "role_task_failed",
                    error = %err,
                    "role task failed"
                );
            }
        });

        self.active_task = Some(ActiveRoleTask {
            target,
            worker_id,
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
        let run_id = task.target.run_id;
        let role = task.target.role;
        let worker_id = task.worker_id;

        if let Err(err) = task.handle.await {
            warn!(
                target: "worker_log",
                run_id,
                node_id = %self.node_id,
                worker_id = %worker_id,
                role = %role,
                event_type = "role_task_join_failed",
                error = %err,
                "role task join failed"
            );
        }

        warn!(
            target: "worker_log",
            run_id,
            node_id = %self.node_id,
            role = %role,
            event_type = "role_task_exited",
            "role task exited; waiting for supervisor reconcile"
        );
    }

    pub(super) async fn stop_current(&mut self) {
        let Some(task) = self.active_task.take() else {
            return;
        };
        let run_id = task.target.run_id;
        let role = task.target.role;
        let worker_id = task.worker_id;

        info!(
            target: "worker_log",
            run_id,
            node_id = %self.node_id,
            worker_id = %worker_id,
            role = %role,
            event_type = "role_stop",
            "stopping role task"
        );

        let _ = task.stop_tx.send(true);

        let mut handle: JoinHandle<()> = task.handle;
        match timeout(ROLE_TASK_SHUTDOWN_TIMEOUT, &mut handle).await {
            Ok(join_result) => {
                if let Err(err) = join_result {
                    warn!(
                        target: "worker_log",
                        run_id,
                        node_id = %self.node_id,
                        worker_id = %worker_id,
                        role = %role,
                        event_type = "role_task_join_failed",
                        error = %err,
                        "role task join failed"
                    );
                }
            }
            Err(_) => {
                warn!(
                    target: "worker_log",
                    run_id,
                    node_id = %self.node_id,
                    worker_id = %worker_id,
                    role = %role,
                    event_type = "role_task_shutdown_timeout",
                    "timed out waiting for role task shutdown; aborting task"
                );
                handle.abort();
            }
        }
    }
}
