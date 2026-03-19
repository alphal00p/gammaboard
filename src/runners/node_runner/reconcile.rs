use super::{
    ActiveRoleRunner, ActiveWorker, NodeRunner, NodeRunnerStore, RoleRunner, RoleTarget,
    evaluator_role_runner::EvaluatorRoleRunner,
    sampler_aggregator_role_runner::SamplerAggregatorRoleRunner,
};
use crate::core::StoreError;
use tracing::{error, info, warn};

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

        if self.current_target() == desired_target {
            return Ok(());
        }

        self.stop_current().await;

        let Some(target) = desired_target else {
            return Ok(());
        };
        if self.blocked_target == Some(target) {
            return Ok(());
        }

        if let Err(err) = self.start(target).await {
            self.note_start_failure(target);
            error!("failed to start role runner: {err}");
        }

        Ok(())
    }

    async fn start(&mut self, target: RoleTarget) -> Result<(), StoreError> {
        let context_span = tracing::span!(
            tracing::Level::TRACE,
            "role_runner_context",
            run_id = target.run_id,
            node_id = %self.node_id,
            role = %target.role
        );
        let role_scope_span = context_span.clone();
        let _role_scope = role_scope_span.enter();
        info!("starting role runner");

        let worker = ActiveWorker::new(
            self.store.clone(),
            self.node_id.clone(),
            target.role,
            target.run_id,
        );
        let runner = self.build_runner(&worker).await?;
        worker.mark_active_with_log().await?;
        self.active_runner = Some(ActiveRoleRunner {
            target,
            worker,
            context_span,
            runner,
        });
        self.note_role_started();
        Ok(())
    }

    async fn build_runner(
        &self,
        worker: &ActiveWorker<S>,
    ) -> Result<Box<dyn RoleRunner>, StoreError> {
        match worker.role {
            crate::core::WorkerRole::Evaluator => {
                Ok(Box::new(EvaluatorRoleRunner::new(worker).await?))
            }
            crate::core::WorkerRole::SamplerAggregator => {
                Ok(Box::new(SamplerAggregatorRoleRunner::new(worker).await?))
            }
        }
    }

    pub(super) fn note_role_started(&mut self) {
        self.failure_target = None;
        self.consecutive_start_failures = 0;
        self.blocked_target = None;
    }

    pub(super) fn note_role_stopped(&mut self) {
        self.failure_target = None;
        self.consecutive_start_failures = 0;
        self.blocked_target = None;
    }

    pub(super) fn note_start_failure(&mut self, target: RoleTarget) {
        if self.failure_target == Some(target) {
            self.consecutive_start_failures = self.consecutive_start_failures.saturating_add(1);
        } else {
            self.failure_target = Some(target);
            self.consecutive_start_failures = 1;
        }
        if self.consecutive_start_failures >= self.config.max_consecutive_start_failures {
            self.blocked_target = Some(target);
            warn!(
                role = %target.role,
                run_id = target.run_id,
                consecutive_failures = self.consecutive_start_failures,
                max_consecutive_start_failures = self.config.max_consecutive_start_failures,
                "aborting role runner restarts after repeated failures; waiting for desired assignment change"
            );
        }
    }

    pub(super) async fn stop_current(&mut self) {
        let Some(mut active_runner) = self.active_runner.take() else {
            return;
        };
        let _role_scope = active_runner.context_span.enter();
        info!("stopping role runner");
        if let Err(err) = active_runner.runner.persist_state().await {
            warn!("failed to persist role runner state on stop: {err}");
        }
        active_runner.worker.mark_inactive_with_log().await;
    }
}
