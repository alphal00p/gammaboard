use crate::core::{RunTask, RunTaskSpec, StoreError};
use crate::runners::sampler_aggregator::{
    SamplerAggregatorRunner, SamplerAggregatorRunnerSnapshot,
};
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::Instrument;
use tracing::{Span, info, warn};

use super::{ActiveWorker, NodeRunnerStore, RoleRunner};

pub(super) struct SamplerAggregatorRoleRunner<S: NodeRunnerStore> {
    worker: ActiveWorker<S>,
    interval: Duration,
    engine_span: Span,
    saved_snapshot: Option<SamplerAggregatorRunnerSnapshot>,
    runner: Option<SamplerAggregatorRunner<S>>,
}

impl<S: NodeRunnerStore> SamplerAggregatorRoleRunner<S> {
    pub(super) async fn new(worker: &ActiveWorker<S>) -> Result<Self, StoreError> {
        let Some(spec) = worker.store.load_run_spec(worker.run_id).await? else {
            warn!("run has no RunSpec; sampler-aggregator not started");
            return Err(StoreError::store("run has no RunSpec"));
        };
        info!("sampler-aggregator worker started");
        Ok(Self {
            worker: worker.clone(),
            interval: Duration::from_millis(spec.sampler_aggregator_runner_params.min_poll_time_ms),
            engine_span: tracing::span!(tracing::Level::TRACE, "sampler_engine_context"),
            saved_snapshot: worker
                .store
                .load_sampler_runner_snapshot(worker.run_id)
                .await?,
            runner: None,
        })
    }

    async fn load_or_activate_task(
        &self,
        open_batch_count: usize,
    ) -> Result<Option<RunTask>, StoreError> {
        if let Some(task) = self
            .worker
            .store
            .load_active_run_task(self.worker.run_id)
            .await?
        {
            return Ok(Some(task));
        }
        if open_batch_count > 0 {
            return Ok(None);
        }
        self.worker
            .store
            .activate_next_run_task(self.worker.run_id)
            .await
    }

    async fn ensure_runner(&mut self, task: &RunTask) -> Result<(), StoreError> {
        if self.runner.as_ref().map(|runner| runner.task_id()) == Some(task.id) {
            return Ok(());
        }
        let Some(spec) = self.worker.store.load_run_spec(self.worker.run_id).await? else {
            return Err(StoreError::store("run has no RunSpec"));
        };
        let is_resuming = self
            .saved_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.task_id == task.id);
        let resumed_snapshot = if is_resuming {
            self.saved_snapshot.take()
        } else {
            None
        };
        let mut runner = SamplerAggregatorRunner::new(
            self.worker.run_id,
            self.worker.node_id.clone(),
            task.clone(),
            self.worker.store.clone(),
            spec.sampler_aggregator_runner_params.clone(),
            spec.point_spec.clone(),
            spec.evaluator.clone(),
            resumed_snapshot,
        )
        .await
        .map_err(|err| StoreError::store(err.to_string()))?;
        if !is_resuming {
            runner
                .persist_state(true)
                .await
                .map_err(|err| StoreError::store(err.to_string()))?;
        }
        self.runner = Some(runner);
        Ok(())
    }

    async fn finish_runner(&mut self) -> Result<(), StoreError> {
        let Some(mut runner) = self.runner.take() else {
            return Ok(());
        };
        runner
            .persist_state(true)
            .await
            .map_err(|err| StoreError::store(err.to_string()))?;
        runner
            .complete_task()
            .await
            .map_err(|err| StoreError::store(err.to_string()))?;
        self.saved_snapshot = None;
        Ok(())
    }

    async fn fail_task(&mut self, task_id: i64, reason: &str) -> Result<(), StoreError> {
        if let Some(mut runner) = self.runner.take() {
            runner
                .fail_task(reason)
                .await
                .map_err(|err| StoreError::store(err.to_string()))?;
        } else {
            self.worker.store.fail_run_task(task_id, reason).await?;
        }
        let cleared = self
            .worker
            .store
            .clear_run_assignments(self.worker.run_id)
            .await?;
        info!(
            run_id = self.worker.run_id,
            task_id,
            assignments_cleared = cleared,
            "run task failed; assignments cleared"
        );
        Ok(())
    }
}

#[async_trait::async_trait(?Send)]
impl<S: NodeRunnerStore> RoleRunner for SamplerAggregatorRoleRunner<S> {
    async fn tick(&mut self) -> Result<bool, StoreError> {
        let open_batch_count = self
            .worker
            .store
            .get_open_batch_count(self.worker.run_id)
            .await?
            .max(0) as usize;

        let Some(task) = self.load_or_activate_task(open_batch_count).await? else {
            if open_batch_count == 0 {
                let cleared = self
                    .worker
                    .store
                    .clear_run_assignments(self.worker.run_id)
                    .await?;
                info!(
                    run_id = self.worker.run_id,
                    assignments_cleared = cleared,
                    "run task queue exhausted; assignments cleared"
                );
                return Ok(true);
            }
            if self.interval > Duration::ZERO {
                sleep(self.interval).await;
            }
            return Ok(false);
        };

        if matches!(task.task, RunTaskSpec::Pause) {
            if open_batch_count == 0 {
                self.worker.store.complete_run_task(task.id).await?;
                let cleared = self
                    .worker
                    .store
                    .clear_run_assignments(self.worker.run_id)
                    .await?;
                info!(
                    run_id = self.worker.run_id,
                    task_id = task.id,
                    assignments_cleared = cleared,
                    "pause task reached; run assignments cleared"
                );
                return Ok(true);
            }
            if self.interval > Duration::ZERO {
                sleep(self.interval).await;
            }
            return Ok(false);
        }

        self.ensure_runner(&task).await?;

        let started = Instant::now();
        let done = match self
            .runner
            .as_mut()
            .expect("runner should exist for non-pause task")
            .tick()
            .instrument(self.engine_span.clone())
            .await
        {
            Ok(done) => done,
            Err(err) => {
                warn!("sampler-aggregator tick failed: {err}");
                self.fail_task(task.id, &err.to_string()).await?;
                return Ok(true);
            }
        };

        if done {
            self.finish_runner().await?;
            return Ok(true);
        }

        let elapsed = started.elapsed();
        if elapsed < self.interval {
            sleep(self.interval - elapsed).await;
        }

        Ok(false)
    }

    async fn persist_state(&mut self) -> Result<(), StoreError> {
        let Some(runner) = self.runner.as_mut() else {
            return Ok(());
        };
        let queue_empty = self
            .worker
            .store
            .get_open_batch_count(self.worker.run_id)
            .await?
            <= 0;
        runner
            .persist_state(queue_empty)
            .await
            .map_err(|err| StoreError::store(err.to_string()))
    }
}
