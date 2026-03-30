use super::{
    ActiveRoleRunner, ActiveWorker, NodeRunner, NodeRunnerStore, RoleTarget,
    role_runner::RoleRunner,
};

use crate::core::{
    BatchTransformConfig, ObservableConfig, RunStageSnapshot, RunTask, RunTaskState, SourceRefSpec,
    StoreError,
};
use crate::runners::{EvaluatorRunner, SamplerAggregatorRunner};
use crate::sampling::StageHandoffOwned;
use tracing::{error, info, warn};

impl<S: NodeRunnerStore> NodeRunner<S> {
    async fn resolve_effective_sampler_context(
        &self,
        worker: &ActiveWorker<S>,
        task: &RunTask,
    ) -> Result<
        (
            crate::core::SamplerAggregatorConfig,
            Vec<BatchTransformConfig>,
            Option<StageHandoffOwned>,
        ),
        StoreError,
    > {
        let latest_snapshot = worker
            .store
            .load_sampler_runner_snapshot(worker.run_id)
            .await?;
        let restored_snapshot = latest_snapshot
            .as_ref()
            .filter(|snapshot| snapshot.task_id == task.id)
            .cloned();
        let base_stage_snapshot = worker
            .store
            .load_latest_stage_snapshot_before_sequence(worker.run_id, i32::MAX)
            .await?;
        let sampler_source_snapshot =
            Self::resolve_source_snapshot(worker, task, task.task.sample_sampler_source()).await?;

        let sampler_config = if let Some(config) = task.task.sampler_config() {
            config
        } else {
            task.task
                .sample_sampler_config()
                .or_else(|| {
                    sampler_source_snapshot
                        .as_ref()
                        .map(|snapshot| snapshot.sampler_aggregator.clone())
                })
                .or_else(|| {
                    base_stage_snapshot
                        .as_ref()
                        .map(|snapshot| snapshot.sampler_aggregator.clone())
                })
                .ok_or_else(|| {
                    StoreError::store(format!(
                        "run {} task {} has no sampler configuration",
                        worker.run_id, task.id
                    ))
                })?
        };

        let batch_transforms = task
            .task
            .batch_transforms_config()
            .or_else(|| {
                sampler_source_snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.batch_transforms.clone())
            })
            .or_else(|| {
                base_stage_snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.batch_transforms.clone())
            })
            .unwrap_or_default();

        let handoff = if let Some(runner_snapshot) = restored_snapshot {
            Some(Self::owned_stage_handoff_from_runner_snapshot(
                runner_snapshot,
            ))
        } else if let Some(snapshot) = sampler_source_snapshot {
            Some(Self::owned_stage_handoff_from_stage_snapshot(snapshot))
        } else {
            match &sampler_config {
                crate::core::SamplerAggregatorConfig::HavanaInference { params } => {
                    let snapshot = match &params.source {
                        crate::sampling::HavanaInferenceSource::Snapshot { snapshot_id } => {
                            worker.store.load_stage_snapshot(*snapshot_id).await?
                        }
                        crate::sampling::HavanaInferenceSource::LatestTrainingSamplerAggregator => {
                            Self::find_latest_havana_snapshot(worker).await?
                        }
                    };
                    if let Some(snapshot) = snapshot {
                        Some(Self::owned_stage_handoff_from_stage_snapshot(snapshot))
                    } else {
                        let reason = "havana_inference sampler requires a havana training or inference snapshot handoff";
                        if let Err(e) = worker.store.fail_run_task(task.id, reason).await {
                            warn!(
                                run_id = worker.run_id,
                                task_id = task.id,
                                error = %e,
                                "failed to persist task failure for activation error"
                            );
                        }
                        if let Err(e) = self
                            .store
                            .clear_desired_assignments_for_run(worker.run_id)
                            .await
                        {
                            warn!(
                                run_id = worker.run_id,
                                error = %e,
                                "failed to clear desired assignments for run after task activation failure"
                            );
                        } else {
                            warn!(
                                run_id = worker.run_id,
                                task_id = task.id,
                                reason,
                                "task activation failed (missing havana snapshot); desired assignments cleared"
                            );
                        }
                        return Err(StoreError::store(reason));
                    }
                }
                _ => base_stage_snapshot.map(Self::owned_stage_handoff_from_stage_snapshot),
            }
        };

        Ok((sampler_config, batch_transforms, handoff))
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
            Err(err) => {
                self.note_start_failure(target);
                error!("failed to start role runner: {err}");
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
        let worker = ActiveWorker::new(
            self.store.clone(),
            self.node_name.clone(),
            self.node_uuid.clone(),
            target.role,
            target.run_id,
        );
        match target.role {
            crate::core::WorkerRole::Evaluator => self.build_evaluator_runner(&worker).await,
            crate::core::WorkerRole::SamplerAggregator => self.build_sampler_runner(&worker).await,
        }
    }

    async fn build_evaluator_runner(
        &self,
        worker: &ActiveWorker<S>,
    ) -> Result<Option<Box<dyn RoleRunner>>, StoreError> {
        let Some(spec) = worker.store.load_run_spec(worker.run_id).await? else {
            warn!("run has no RunSpec; evaluator not started");
            return Ok(None);
        };
        let evaluator = spec
            .evaluator
            .build()
            .map_err(|err| StoreError::store(format!("failed to build evaluator: {err}")))?;
        info!("evaluator worker started");

        let runner = EvaluatorRunner::new(
            worker.store.clone(),
            worker.run_id,
            self.node_name.clone(),
            self.node_uuid.clone(),
            evaluator,
            spec.domain.clone(),
            spec.evaluator_runner_params.clone(),
        );

        Ok(Some(Box::new(runner)))
    }

    async fn load_or_activate_sampler_task(
        &self,
        worker: &ActiveWorker<S>,
        open_batch_count: usize,
    ) -> Result<Option<RunTask>, StoreError> {
        if let Some(task) = worker.store.load_active_run_task(worker.run_id).await? {
            return Ok(Some(task));
        }
        if open_batch_count > 0 {
            return Ok(None);
        }
        worker.store.activate_next_run_task(worker.run_id).await
    }

    async fn build_sampler_runner(
        &self,
        worker: &ActiveWorker<S>,
    ) -> Result<Option<Box<dyn RoleRunner>>, StoreError> {
        let Some(spec) = worker.store.load_run_spec(worker.run_id).await? else {
            warn!("run has no RunSpec; sampler-aggregator not started");
            return Ok(None);
        };

        let open_batch_count = worker
            .store
            .get_open_batch_count(worker.run_id)
            .await?
            .max(0) as usize;
        let Some(task) = self
            .load_or_activate_sampler_task(worker, open_batch_count)
            .await?
        else {
            if open_batch_count == 0 {
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

        let latest_snapshot = worker
            .store
            .load_sampler_runner_snapshot(worker.run_id)
            .await?;
        let initial_batch_size_hint = latest_snapshot
            .as_ref()
            .filter(|snapshot| snapshot.task_id != task.id)
            .map(|snapshot| {
                snapshot.reduced_carryover_batch_size(
                    spec.sampler_aggregator_runner_params.max_batch_size,
                )
            });
        let restored_snapshot = latest_snapshot
            .as_ref()
            .filter(|snapshot| snapshot.task_id == task.id)
            .cloned();

        let (sampler_config, batch_transforms, handoff) = self
            .resolve_effective_sampler_context(worker, &task)
            .await?;
        let base_stage_snapshot = worker
            .store
            .load_latest_stage_snapshot_before_sequence(worker.run_id, i32::MAX)
            .await?;
        let observable_source_snapshot =
            Self::resolve_source_snapshot(worker, &task, task.task.sample_observable_source())
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
                // Sampler build failed at activation time. Persist a task failure and pause the run
                // (clear desired assignments) so an operator can inspect and add a replacement.
                let reason = format!("failed to build sampler: {err}");
                if let Err(e) = worker.store.fail_run_task(task.id, &reason).await {
                    warn!(
                        run_id = worker.run_id,
                        task_id = task.id,
                        error = %e,
                        "failed to persist task failure after sampler build error"
                    );
                }
                if let Err(e) = self
                    .store
                    .clear_desired_assignments_for_run(worker.run_id)
                    .await
                {
                    warn!(
                        run_id = worker.run_id,
                        error = %e,
                        "failed to clear desired assignments for run after sampler build error"
                    );
                } else {
                    warn!(
                        run_id = worker.run_id,
                        task_id = task.id,
                        error = %err,
                        failure_reason = %reason,
                        "task activation failed during sampler build; desired assignments cleared"
                    );
                }
                return Ok(None);
            }
        };

        let new_observable_config = task
            .task
            .new_observable_config()
            .map_err(|err| StoreError::store(err.to_string()))?;

        let observable_state = if let Some(snapshot) = restored_snapshot.as_ref() {
            snapshot.observable_state.clone()
        } else if let Some(source_snapshot) = observable_source_snapshot.as_ref() {
            source_snapshot
                .observable_state
                .clone()
                .unwrap_or_else(|| crate::evaluation::ObservableState::empty_scalar())
        } else if let Some(config) = new_observable_config {
            Self::observable_state_from_config(config)
        } else if let Some(snapshot) = base_stage_snapshot.as_ref() {
            snapshot
                .observable_state
                .clone()
                .unwrap_or_else(|| crate::evaluation::ObservableState::empty_scalar())
        } else {
            crate::evaluation::ObservableState::empty_scalar()
        };

        let run_progress = worker.store.load_run_sample_progress(worker.run_id).await?;

        let initial_batch_size =
            initial_batch_size_hint.unwrap_or(spec.sampler_aggregator_runner_params.max_batch_size);

        let restored_snapshot_for_runner = restored_snapshot.clone();
        let task_for_runner = task.clone();

        let mut runner = SamplerAggregatorRunner::new(
            worker.store.clone(),
            worker.run_id,
            self.node_name.clone(),
            task_for_runner,
            sampler,
            observable_state,
            sampler_config,
            batch_transforms,
            spec.sampler_aggregator_runner_params.clone(),
            initial_batch_size,
            restored_snapshot_for_runner,
            run_progress,
        );

        if runner.task_state().state == RunTaskState::Active {
            runner
                .persist_state()
                .await
                .map_err(|err| StoreError::store(err.to_string()))?;
        }
        info!("sampler-aggregator worker started");
        Ok(Some(Box::new(runner)))
    }

    async fn resolve_source_snapshot(
        worker: &ActiveWorker<S>,
        task: &RunTask,
        source: Option<SourceRefSpec>,
    ) -> Result<Option<RunStageSnapshot>, StoreError> {
        match source {
            Some(SourceRefSpec::Latest) => {
                worker
                    .store
                    .load_latest_stage_snapshot_before_sequence(worker.run_id, task.sequence_nr)
                    .await
            }
            Some(SourceRefSpec::FromName(source_task_name)) => {
                let source_task = worker
                    .store
                    .list_run_tasks(worker.run_id)
                    .await?
                    .into_iter()
                    .find(|candidate| candidate.name == source_task_name)
                    .ok_or_else(|| {
                        StoreError::store(format!(
                            "task {} references source task '{}' but no such task exists in run {}",
                            task.id, source_task_name, worker.run_id
                        ))
                    })?;
                if source_task.sequence_nr >= task.sequence_nr {
                    return Err(StoreError::store(format!(
                        "task {} references source task '{}' which is not prior in sequence",
                        task.id, source_task_name
                    )));
                }
                let snapshot = worker
                    .store
                    .load_latest_stage_snapshot_before_sequence(
                        worker.run_id,
                        source_task.sequence_nr + 1,
                    )
                    .await?
                    .ok_or_else(|| {
                        StoreError::store(format!(
                            "task {} source task '{}' has no queue-empty stage snapshot",
                            task.id, source_task_name
                        ))
                    })?;
                if snapshot.task_id != Some(source_task.id) {
                    return Err(StoreError::store(format!(
                        "task {} source task '{}' has no queue-empty stage snapshot",
                        task.id, source_task_name
                    )));
                }
                Ok(Some(snapshot))
            }
            None => Ok(None),
        }
    }

    async fn find_latest_havana_snapshot(
        worker: &ActiveWorker<S>,
    ) -> Result<Option<RunStageSnapshot>, StoreError> {
        let mut search_seq = i32::MAX;
        loop {
            let Some(snapshot) = worker
                .store
                .load_latest_stage_snapshot_before_sequence(worker.run_id, search_seq)
                .await?
            else {
                return Ok(None);
            };
            if snapshot.sampler_snapshot.contains_havana_grid() {
                return Ok(Some(snapshot));
            }
            let prev_seq = snapshot.sequence_nr.unwrap_or(0);
            if prev_seq <= 0 {
                return Ok(None);
            }
            search_seq = prev_seq;
        }
    }

    fn owned_stage_handoff_from_stage_snapshot(snapshot: RunStageSnapshot) -> StageHandoffOwned {
        StageHandoffOwned {
            sampler_snapshot: Some(snapshot.sampler_snapshot),
            observable_state: snapshot.observable_state,
        }
    }

    fn owned_stage_handoff_from_runner_snapshot(
        snapshot: crate::runners::sampler_aggregator::SamplerAggregatorRunnerSnapshot,
    ) -> StageHandoffOwned {
        StageHandoffOwned {
            sampler_snapshot: Some(snapshot.sampler_snapshot),
            observable_state: Some(snapshot.observable_state),
        }
    }

    fn observable_state_from_config(
        config: ObservableConfig,
    ) -> crate::evaluation::ObservableState {
        match config {
            crate::core::ObservableConfig::Scalar => {
                crate::evaluation::ObservableState::empty_scalar()
            }
            crate::core::ObservableConfig::Complex => {
                crate::evaluation::ObservableState::empty_complex()
            }
            crate::core::ObservableConfig::FullScalar => {
                crate::evaluation::ObservableState::empty_full_scalar()
            }
            crate::core::ObservableConfig::FullComplex => {
                crate::evaluation::ObservableState::empty_full_complex()
            }
        }
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
        if matches!(
            role,
            crate::core::WorkerRole::SamplerAggregator | crate::core::WorkerRole::Evaluator
        ) {
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
        if let Err(err) = active_runner.runner.persist_state().await {
            warn!("failed to persist role runner state on stop: {err}");
        }
        active_runner.worker.mark_inactive_with_log().await;
    }
}
