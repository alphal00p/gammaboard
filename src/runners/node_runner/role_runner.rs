use crate::core::StoreError;
use crate::core::StoreResultExt;
use crate::runners::{EvaluatorRunner, EvaluatorRunnerError, RunnerError, SamplerAggregatorRunner};
use async_trait::async_trait;
use std::time::Duration;

#[async_trait(?Send)]
pub(super) trait RoleRunner {
    async fn tick(&mut self) -> Result<bool, StoreError>;
    async fn stop(&mut self) -> Result<(), StoreError>;
    fn min_tick_time(&self) -> Duration;
}

#[async_trait(?Send)]
impl<S: crate::core::EvaluatorWorkerStore + Clone + Send + Sync + 'static> RoleRunner
    for EvaluatorRunner<S>
{
    async fn tick(&mut self) -> Result<bool, StoreError> {
        match EvaluatorRunner::tick(self).await {
            Ok(()) => Ok(false),
            Err(EvaluatorRunnerError::Store(err)) => Err(err),
            Err(err) => Err(StoreError::store(err.to_string())),
        }
    }

    async fn stop(&mut self) -> Result<(), StoreError> {
        match EvaluatorRunner::stop(self).await {
            Ok(()) => Ok(()),
            Err(EvaluatorRunnerError::Store(err)) => Err(err),
            Err(err) => Err(StoreError::store(err.to_string())),
        }
    }

    fn min_tick_time(&self) -> Duration {
        Duration::from_millis(self.params().min_tick_time_ms)
    }
}

#[async_trait(?Send)]
impl<S: crate::core::SamplerWorkerStore + Clone + Send + Sync + 'static> RoleRunner
    for SamplerAggregatorRunner<S>
{
    async fn tick(&mut self) -> Result<bool, StoreError> {
        match SamplerAggregatorRunner::tick(self).await {
            Ok(done) => {
                if done {
                    self.complete_task().await.store_err()?;
                    return Ok(true);
                }
                Ok(false)
            }
            Err(RunnerError::Store(err)) if err.is_database_error() => Err(err),
            Err(err) => {
                self.fail_task(&err.to_string()).await.store_err()?;
                match err {
                    RunnerError::Store(err) => Err(err),
                    err => Err(StoreError::store(err.to_string())),
                }
            }
        }
    }

    async fn stop(&mut self) -> Result<(), StoreError> {
        match SamplerAggregatorRunner::persist_state(self).await {
            Ok(()) => Ok(()),
            Err(RunnerError::Store(err)) => Err(err),
            Err(err) => Err(StoreError::store(err.to_string())),
        }
    }

    fn min_tick_time(&self) -> Duration {
        Duration::from_millis(self.params().min_tick_time_ms)
    }
}
