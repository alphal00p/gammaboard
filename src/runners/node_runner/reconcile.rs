use super::{
    ActiveRoleRunner, ActiveWorker, NodeRunner, NodeRunnerStore, RoleTarget,
    role_runner::RoleRunner,
};

use crate::api::stage::resolve_task_source_snapshot;
use crate::core::{
    AggregationStore, BatchTransformConfig, RunStageSnapshot, RunTask, RunTaskStore, StoreError,
    WorkQueueStore,
};
use crate::runners::{
    EvaluatorRunner, SamplerAggregatorRunner,
    stage_context::{HAVANA_HANDOFF_REQUIRED_ERROR, resolve_stage_context},
};
use crate::sampling::StageHandoffOwned;
use tracing::{error, info, warn};

impl<S: NodeRunnerStore> NodeRunner<S> {
    async fn fail_task_activation_and_pause_run(
        &self,
        run_id: i32,
        task_id: i64,
        reason: &str,
        failure_log: &str,
    ) {
        if let Err(err) = self.store.fail_run_task(task_id, reason).await {
            warn!(
                run_id,
                task_id,
                error = %err,
                "{failure_log}: failed to persist task failure"
            );
        }
        match self.store.clear_desired_assignments_for_run(run_id).await {
            Ok(cleared) => warn!(
                run_id,
                task_id,
                reason,
                assignments_cleared = cleared,
                "{failure_log}: desired assignments cleared"
            ),
            Err(err) => warn!(
                run_id,
                task_id,
                reason,
                error = %err,
                "{failure_log}: failed to clear desired assignments"
            ),
        }
    }

    fn missing_accumulator_configuration_error(run_id: i32, task_id: i64) -> StoreError {
        StoreError::store(format!(
            "run {} task {} has no accumulator configuration",
            run_id, task_id
        ))
    }

    fn resolve_observable_state_for_sampler_task(
        run_id: i32,
        task_id: i64,
        restored_snapshot: Option<&crate::runners::sampler_aggregator::SamplerAggregatorCheckpoint>,
        accumulator_source_snapshot: Option<&RunStageSnapshot>,
        base_stage_snapshot: Option<&RunStageSnapshot>,
        new_accumulator_config: Option<crate::core::AccumulatorConfig>,
    ) -> Result<crate::evaluation::AccumulatorState, StoreError> {
        if let Some(snapshot) = restored_snapshot {
            return Ok(snapshot.observable_state.clone());
        }
        if let Some(source_snapshot) = accumulator_source_snapshot {
            return source_snapshot.observable_state.clone().ok_or_else(|| {
                StoreError::store(format!(
                    "run {} task {} source snapshot has no accumulator state",
                    run_id, task_id
                ))
            });
        }
        if let Some(config) = new_accumulator_config {
            return Ok(crate::evaluation::AccumulatorState::from_config(&config));
        }
        if let Some(base_snapshot) = base_stage_snapshot {
            return base_snapshot
                .observable_state
                .clone()
                .ok_or_else(|| Self::missing_accumulator_configuration_error(run_id, task_id));
        }
        Err(Self::missing_accumulator_configuration_error(
            run_id, task_id,
        ))
    }

    async fn resolve_effective_sampler_context_with_store(
        &self,
        store: &crate::PgStore,
        run_id: i32,
        task: &RunTask,
    ) -> Result<
        (
            crate::core::SamplerAggregatorConfig,
            Vec<BatchTransformConfig>,
            Option<StageHandoffOwned>,
        ),
        StoreError,
    > {
        let latest_snapshot = store.load_sampler_checkpoint(run_id).await?;
        let restored_snapshot = latest_snapshot
            .as_ref()
            .filter(|snapshot| snapshot.task_id == task.id)
            .cloned();
        match resolve_stage_context(store, run_id, task, task.sequence_nr, restored_snapshot).await
        {
            Ok(resolved) => Ok((
                resolved.sampler_config,
                resolved.batch_transforms,
                resolved.handoff,
            )),
            Err(err) if err.to_string() == HAVANA_HANDOFF_REQUIRED_ERROR => {
                let reason = err.to_string();
                self.fail_task_activation_and_pause_run(
                    run_id,
                    task.id,
                    &reason,
                    "task activation failed (missing havana snapshot)",
                )
                .await;
                Err(err)
            }
            Err(err) => Err(err),
        }
    }

    pub(super) async fn resolve_desired_target(&self) -> Result<Option<RoleTarget>, StoreError> {
        let assignment = self.store.get_desired_assignment(&self.node_name).await?;
        Ok(assignment.map(|assignment| RoleTarget {
            role: assignment.role,
            run_id: assignment.run_id,
        }))
    }

    pub(super) async fn reconcile(
        &mut self,
        desired_target: Option<RoleTarget>,
    ) -> Result<(), StoreError> {
        self.retry_state
            .reset_for_desired_target_change(desired_target);

        if self.current_target() == desired_target {
            return Ok(());
        }

        self.reset_reconcile_backoff();

        self.stop_current().await;

        let Some((target, runner)) = self.build_reconciled_runner(desired_target).await? else {
            return Ok(());
        };
        self.start(target, runner).await?;

        Ok(())
    }

    async fn build_reconciled_runner(
        &mut self,
        desired_target: Option<RoleTarget>,
    ) -> Result<Option<(RoleTarget, Box<dyn RoleRunner>)>, StoreError> {
        let Some(target) = desired_target else {
            return Ok(None);
        };
        if self.retry_state.is_blocked(target) {
            return Ok(None);
        }

        match self.build_runner_for_target(target).await {
            Ok(Some(runner)) => Ok(Some((target, runner))),
            Ok(None) => Ok(None),
            Err(err) if err.is_database_error() => Err(err),
            Err(err) => {
                self.note_start_failure(target);
                error!(
                    role = %target.role,
                    run_id = target.run_id,
                    error = %err,
                    "failed to start role runner"
                );
                Ok(None)
            }
        }
    }

    async fn start(
        &mut self,
        target: RoleTarget,
        runner: Box<dyn RoleRunner>,
    ) -> Result<(), StoreError> {
        let context_span = tracing::span!(
            tracing::Level::TRACE,
            "role_runner_context",
            run_id = target.run_id,
            node_name = %self.node_name,
            node_uuid = %self.node_uuid,
            role = %target.role
        );
        let role_scope_span = context_span.clone();
        let _role_scope = role_scope_span.enter();
        info!("starting role runner");

        let worker = ActiveWorker::new(
            self.store.clone(),
            self.node_name.clone(),
            self.node_uuid.clone(),
            target.role,
            target.run_id,
        );
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

    async fn build_runner_for_target(
        &self,
        target: RoleTarget,
    ) -> Result<Option<Box<dyn RoleRunner>>, StoreError> {
        match target.role {
            crate::core::WorkerRole::Evaluator => self.build_evaluator_runner(target).await,
            crate::core::WorkerRole::SamplerAggregator => self.build_sampler_runner(target).await,
        }
    }

    async fn build_evaluator_runner(
        &self,
        target: RoleTarget,
    ) -> Result<Option<Box<dyn RoleRunner>>, StoreError> {
        let Some(spec) = self.store.load_run_spec(target.run_id).await? else {
            warn!("run has no RunSpec; evaluator not started");
            return Ok(None);
        };
        let role_store = self
            .init_role_store(spec.evaluator_runner_params.db_pool_size)
            .await?;
        let evaluator = spec
            .evaluator
            .build()
            .map_err(|err| StoreError::store(format!("failed to build evaluator: {err}")))?;
        info!("evaluator worker started");

        let runner = EvaluatorRunner::new(
            role_store,
            target.run_id,
            self.node_name.clone(),
            self.node_uuid.clone(),
            evaluator,
            spec.domain.clone(),
            spec.evaluator_runner_params.clone(),
            spec.sampler_aggregator_runner_params
                .queue
                .max_batch_retries,
        );

        Ok(Some(Box::new(runner)))
    }

    async fn load_or_activate_sampler_task(
        &self,
        store: &crate::PgStore,
        run_id: i32,
        open_batch_count: usize,
    ) -> Result<Option<RunTask>, StoreError> {
        loop {
            if let Some(task) = store.load_active_run_task(run_id).await? {
                if self
                    .apply_immediate_stage_update_task(store, run_id, &task)
                    .await?
                {
                    continue;
                }
                return Ok(Some(task));
            }
            if open_batch_count > 0 {
                return Ok(None);
            }
            let Some(task) = store.activate_next_run_task(run_id).await? else {
                return Ok(None);
            };
            if self
                .apply_immediate_stage_update_task(store, run_id, &task)
                .await?
            {
                continue;
            }
            return Ok(Some(task));
        }
    }

    async fn apply_immediate_stage_update_task(
        &self,
        store: &crate::PgStore,
        run_id: i32,
        task: &RunTask,
    ) -> Result<bool, StoreError> {
        let crate::core::RunTaskSpec::SetAccumulator { accumulator } = &task.task else {
            return Ok(false);
        };

        let base_stage_snapshot = store
            .load_latest_stage_snapshot_before_sequence(run_id, task.sequence_nr)
            .await?;
        let base_stage_snapshot = base_stage_snapshot.ok_or_else(|| {
            StoreError::store(format!(
                "run {} task {} cannot resolve base stage snapshot",
                run_id, task.id
            ))
        })?;

        store
            .save_run_stage_snapshot(&RunStageSnapshot {
                id: None,
                run_id,
                task_id: Some(task.id),
                name: task.name.clone(),
                sequence_nr: Some(task.sequence_nr),
                queue_empty: true,
                sampler_snapshot: base_stage_snapshot.sampler_snapshot,
                observable_state: Some(crate::evaluation::AccumulatorState::from_config(
                    accumulator,
                )),
                sampler_aggregator: base_stage_snapshot.sampler_aggregator,
                batch_transforms: base_stage_snapshot.batch_transforms,
            })
            .await?;
        store.complete_run_task(task.id).await?;
        info!(
            run_id,
            task_id = task.id,
            task_name = %task.name,
            accumulator = ?accumulator,
            "applied set_accumulator task"
        );
        Ok(true)
    }

    async fn build_sampler_runner(
        &self,
        target: RoleTarget,
    ) -> Result<Option<Box<dyn RoleRunner>>, StoreError> {
        let Some(spec) = self.store.load_run_spec(target.run_id).await? else {
            warn!("run has no RunSpec; sampler-aggregator not started");
            return Ok(None);
        };
        let role_store = self
            .init_role_store(spec.sampler_aggregator_runner_params.db_pool_size)
            .await?;
        let worker = ActiveWorker::new(
            self.store.clone(),
            self.node_name.clone(),
            self.node_uuid.clone(),
            target.role,
            target.run_id,
        );

        let queue_counts = role_store
            .get_batch_queue_counts(worker.run_id, None)
            .await?;
        let unfinished_batch_count = queue_counts
            .pending
            .saturating_add(queue_counts.claimed)
            .max(0) as usize;
        let Some(task) = self
            .load_or_activate_sampler_task(&role_store, worker.run_id, unfinished_batch_count)
            .await?
        else {
            if unfinished_batch_count == 0 {
                let cleared = self
                    .store
                    .clear_desired_assignments_for_run(worker.run_id)
                    .await?;
                info!(
                    run_id = worker.run_id,
                    assignments_cleared = cleared,
                    "run task queue exhausted; desired assignments cleared"
                );
            }
            return Ok(None);
        };

        let latest_snapshot = role_store.load_sampler_checkpoint(worker.run_id).await?;
        let initial_batch_size_hint = latest_snapshot
            .as_ref()
            .filter(|snapshot| snapshot.task_id != task.id)
            .map(|snapshot| {
                snapshot.reduced_carryover_batch_size(
                    spec.sampler_aggregator_runner_params.queue.max_batch_size,
                )
            });
        let restored_snapshot = latest_snapshot
            .as_ref()
            .filter(|snapshot| snapshot.task_id == task.id)
            .cloned();

        let (sampler_config, batch_transforms, handoff) = self
            .resolve_effective_sampler_context_with_store(&role_store, worker.run_id, &task)
            .await?;
        let base_stage_snapshot = role_store
            .load_latest_stage_snapshot_before_sequence(worker.run_id, task.sequence_nr)
            .await?;
        let accumulator_source_snapshot = resolve_task_source_snapshot(
            &role_store,
            worker.run_id,
            &task,
            task.task.sample_accumulator_source(),
        )
        .await?;

        let sample_budget = task
            .task
            .nr_expected_samples()
            .and_then(|value| usize::try_from(value).ok());

        let sampler = match sampler_config.build(
            spec.domain.clone(),
            sample_budget,
            handoff.as_ref().map(StageHandoffOwned::as_ref),
        ) {
            Ok(s) => s,
            Err(err) => {
                let reason = format!("failed to build sampler: {err}");
                self.fail_task_activation_and_pause_run(
                    worker.run_id,
                    task.id,
                    &reason,
                    "task activation failed during sampler build",
                )
                .await;
                return Ok(None);
            }
        };

        let new_accumulator_config = task
            .task
            .new_accumulator_config()
            .map_err(|err| StoreError::store(err.to_string()))?;

        let observable_state = Self::resolve_observable_state_for_sampler_task(
            worker.run_id,
            task.id,
            restored_snapshot.as_ref(),
            accumulator_source_snapshot.as_ref(),
            base_stage_snapshot.as_ref(),
            new_accumulator_config,
        )?;

        let initial_batch_size = initial_batch_size_hint
            .unwrap_or(spec.sampler_aggregator_runner_params.queue.max_batch_size);
        let run_progress = role_store
            .load_run_sample_progress(worker.run_id)
            .await?
            .unwrap_or_default();

        let base_queue_config = spec.sampler_aggregator_runner_params.queue.clone();
        let mut effective_sampler_runner_params = spec.sampler_aggregator_runner_params.clone();
        if let Some(queue_tuning) = task.task.sample_queue_tuning() {
            effective_sampler_runner_params
                .queue
                .apply_tuning(queue_tuning);
        }

        let restored_snapshot_for_runner = restored_snapshot.clone();
        let task_for_runner = task.clone();

        let runner = SamplerAggregatorRunner::new(
            role_store,
            worker.run_id,
            self.node_name.clone(),
            task_for_runner,
            sampler,
            observable_state,
            sampler_config,
            batch_transforms,
            effective_sampler_runner_params,
            base_queue_config,
            initial_batch_size,
            run_progress,
            restored_snapshot_for_runner,
        );

        info!("sampler-aggregator worker started");
        Ok(Some(Box::new(runner)))
    }

    pub(super) fn note_role_started(&mut self) {
        self.retry_state.clear();
        self.reset_reconcile_backoff();
    }

    pub(super) fn note_start_failure(&mut self, target: RoleTarget) {
        if self
            .retry_state
            .note_failure(target, self.config.max_consecutive_start_failures)
        {
            warn!(
                role = %target.role,
                run_id = target.run_id,
                consecutive_failures = self.retry_state.consecutive_failures,
                max_consecutive_start_failures = self.config.max_consecutive_start_failures,
                "aborting role runner restarts after repeated failures; waiting for desired assignment change"
            );
        }
    }

    pub(super) async fn finish_current_assignment(&mut self) -> Result<(), StoreError> {
        self.retry_state.clear();
        self.reset_reconcile_backoff();
        self.stop_current().await;
        Ok(())
    }

    pub(super) async fn fail_current_assignment(
        &mut self,
        target: RoleTarget,
        err: &StoreError,
    ) -> Result<(), StoreError> {
        self.note_start_failure(target);
        self.reset_reconcile_backoff();
        self.stop_current().await;
        let role = target.role;
        if matches!(role, crate::core::WorkerRole::Evaluator) {
            warn!(
                run_id = target.run_id,
                role = %role,
                error = %err,
                "evaluator role failed; worker will retry without failing the run"
            );
            return Ok(());
        }

        if matches!(role, crate::core::WorkerRole::SamplerAggregator) {
            if let Some(task) = self.store.load_active_run_task(target.run_id).await? {
                let reason = format!("{role} role failed: {err}");
                if let Err(fail_err) = self.store.fail_run_task(task.id, &reason).await {
                    warn!(
                        run_id = target.run_id,
                        task_id = task.id,
                        error = %fail_err,
                        "failed to persist task failure after role error"
                    );
                }
            }

            let cleared = self
                .store
                .clear_desired_assignments_for_run(target.run_id)
                .await?;
            warn!(
                run_id = target.run_id,
                role = %role,
                assignments_cleared = cleared,
                error = %err,
                "role failed; desired assignments cleared"
            );
        }
        Ok(())
    }

    pub(super) async fn stop_current(&mut self) {
        let Some(mut active_runner) = self.active_runner.take() else {
            return;
        };
        let _role_scope = active_runner.context_span.enter();
        info!("stopping role runner");
        if let Err(err) = active_runner.runner.stop().await {
            warn!("failed to stop role runner cleanly: {err}");
        }
        active_runner.worker.mark_inactive_with_log().await;
    }
}
