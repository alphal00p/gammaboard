use crate::core::{
    AggregationStore, ControlPlaneStore, RunReadStore, RunSpecStore, RunStageSnapshot,
    RunTaskStore, StoreError, WorkQueueStore,
};
use crate::runners::hyperparameter_tuning::HyperparameterTuningRunner;
use crate::runners::parameter_scan::ParameterScanRunner;
use std::time::Duration;
use tokio::{sync::watch, time::sleep};
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
pub struct TaskControlLoopConfig {
    pub tick_interval: Duration,
    pub error_retry_interval: Duration,
}

impl Default for TaskControlLoopConfig {
    fn default() -> Self {
        Self {
            tick_interval: Duration::from_millis(500),
            error_retry_interval: Duration::from_secs(2),
        }
    }
}

pub struct TaskControlLoop<S> {
    store: S,
    config: TaskControlLoopConfig,
    node_name: String,
}

impl<S> TaskControlLoop<S> {
    pub fn new(store: S, config: TaskControlLoopConfig, node_name: String) -> Self {
        Self {
            store,
            config,
            node_name,
        }
    }
}

impl<S> TaskControlLoop<S>
where
    S: AggregationStore
        + ControlPlaneStore
        + RunReadStore
        + RunSpecStore
        + RunTaskStore
        + WorkQueueStore
        + Clone
        + Send
        + Sync
        + 'static,
{
    pub async fn run(self, mut shutdown: watch::Receiver<bool>) {
        loop {
            if *shutdown.borrow() {
                break;
            }
            let sleep_for = match self.tick().await {
                Ok(()) => self.config.tick_interval,
                Err(err) => {
                    warn!(error = %err, "task control loop tick failed");
                    self.config.error_retry_interval
                }
            };
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        break;
                    }
                }
                _ = sleep(sleep_for) => {}
            }
        }
        info!("task control loop stopped");
    }

    async fn tick(&self) -> Result<(), StoreError> {
        if !self.is_current_leader().await? {
            return Ok(());
        }
        let mut first_error = None;
        for run in self.store.get_all_runs().await? {
            if let Err(err) = self.reconcile_run(run.run_id).await {
                warn!(
                    run_id = run.run_id,
                    error = %err,
                    "task control loop failed to reconcile run"
                );
                first_error.get_or_insert(err);
            }
        }
        match first_error {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    async fn is_current_leader(&self) -> Result<bool, StoreError> {
        let leader = self
            .store
            .list_nodes(None)
            .await?
            .into_iter()
            .map(|node| node.name)
            .min();
        Ok(leader.as_deref() == Some(self.node_name.as_str()))
    }

    async fn reconcile_run(&self, run_id: i32) -> Result<(), StoreError> {
        let mut completed_control_task = false;
        if let Some(task) = self.store.load_active_run_task(run_id).await? {
            if task.task.runs_in_control_plane() {
                self.reconcile_control_task(run_id, task).await?;
                if self.store.load_active_run_task(run_id).await?.is_some() {
                    return Ok(());
                }
                completed_control_task = true;
            } else {
                return Ok(());
            }
        }

        let queue_counts = self.store.get_batch_queue_counts(run_id, None).await?;
        let unfinished_batches = queue_counts.pending.saturating_add(queue_counts.claimed);
        if unfinished_batches > 0 {
            return Ok(());
        }

        let Some(run) = self.store.get_run_progress(run_id).await? else {
            return Ok(());
        };
        if !completed_control_task
            && run.desired_assignment_count == 0
            && run.active_worker_count == 0
        {
            return Ok(());
        }

        loop {
            let Some(task) = self.store.activate_next_run_task(run_id).await? else {
                let cleared = self.store.clear_desired_assignments_for_run(run_id).await?;
                if cleared > 0 {
                    debug!(
                        run_id,
                        assignments_cleared = cleared,
                        "run task queue exhausted"
                    );
                }
                return Ok(());
            };
            if task.task.runs_on_sampler_worker() {
                return Ok(());
            }
            self.reconcile_control_task(run_id, task).await?;
            if self.store.load_active_run_task(run_id).await?.is_some() {
                return Ok(());
            }
        }
    }

    async fn reconcile_control_task(
        &self,
        run_id: i32,
        task: crate::core::RunTask,
    ) -> Result<(), StoreError> {
        match &task.task {
            crate::core::RunTaskSpec::SetAccumulator { accumulator } => {
                let base_stage_snapshot = self
                    .store
                    .load_latest_stage_snapshot_before_sequence(run_id, task.sequence_nr)
                    .await?
                    .ok_or_else(|| {
                        StoreError::store(format!(
                            "run {} task {} cannot resolve base stage snapshot",
                            run_id, task.id
                        ))
                    })?;
                self.store
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
                        evaluator: base_stage_snapshot.evaluator,
                        sampler_aggregator: base_stage_snapshot.sampler_aggregator,
                        batch_transforms: base_stage_snapshot.batch_transforms,
                    })
                    .await?;
                self.store.complete_run_task(task.id).await?;
                info!(
                    run_id,
                    task_id = task.id,
                    task_name = %task.name,
                    "applied control-plane set_accumulator task"
                );
            }
            crate::core::RunTaskSpec::ParameterScan { .. } => {
                let mut runner = ParameterScanRunner::new(self.store.clone(), run_id, task);
                runner.tick().await?;
            }
            crate::core::RunTaskSpec::HyperparameterTuning { .. } => {
                let mut runner = HyperparameterTuningRunner::new(self.store.clone(), run_id, task);
                runner.tick().await?;
            }
            _ => {}
        }
        Ok(())
    }
}
