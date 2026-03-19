use crate::core::StoreError;
use crate::runners::evaluator::EvaluatorRunner;
use std::time::Duration;
use tokio::time::sleep;
use tracing::Instrument;
use tracing::{Span, info, warn};

use super::{ActiveWorker, NodeRunnerStore, RoleRunner};

pub(super) struct EvaluatorRoleRunner<S: NodeRunnerStore> {
    runner: EvaluatorRunner<S>,
    engine_span: Span,
    idle_backoff: Duration,
}

impl<S: NodeRunnerStore> EvaluatorRoleRunner<S> {
    pub(super) async fn new(worker: &ActiveWorker<S>) -> Result<Self, StoreError> {
        let Some(spec) = worker.store.load_run_spec(worker.run_id).await? else {
            warn!("run has no RunSpec; evaluator not started");
            return Err(StoreError::store("run has no RunSpec"));
        };

        let engine_span = tracing::span!(tracing::Level::TRACE, "evaluator_engine_context");
        let evaluator = {
            let _engine_scope = engine_span.enter();
            spec.evaluator
                .build()
                .map_err(|err| StoreError::store(format!("failed to build evaluator: {err}")))?
        };

        info!("evaluator worker started");

        Ok(Self {
            runner: EvaluatorRunner::new(
                worker.run_id,
                worker.node_id.clone(),
                evaluator,
                spec.point_spec.clone(),
                Duration::from_millis(
                    spec.evaluator_runner_params
                        .performance_snapshot_interval_ms,
                ),
                worker.store.clone(),
            ),
            engine_span,
            idle_backoff: Duration::from_millis(spec.evaluator_runner_params.min_loop_time_ms),
        })
    }
}

#[async_trait::async_trait(?Send)]
impl<S: NodeRunnerStore> RoleRunner for EvaluatorRoleRunner<S> {
    async fn tick(&mut self) -> Result<bool, StoreError> {
        match self
            .runner
            .tick()
            .instrument(self.engine_span.clone())
            .await
        {
            Ok(tick) => {
                if tick.processed_samples == 0 && self.idle_backoff > Duration::ZERO {
                    sleep(self.idle_backoff).await;
                }
            }
            Err(err) => {
                warn!("evaluator tick failed: {err}");
                if self.idle_backoff > Duration::ZERO {
                    sleep(self.idle_backoff).await;
                }
            }
        }
        Ok(false)
    }

    async fn persist_state(&mut self) -> Result<(), StoreError> {
        Ok(())
    }
}
