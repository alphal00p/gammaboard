use crate::core::StoreError;
use crate::runners::{
    EvaluatorRunner, EvaluatorRunnerError, RunnerError, SamplerAggregatorRunner,
    parameter_scan::ParameterScanRunner,
};
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
                    self.complete_task()
                        .await
                        .map_err(|err| StoreError::store(err.to_string()))?;
                    return Ok(true);
                }
                Ok(false)
            }
            Err(RunnerError::Store(err)) if err.is_database_error() => Err(err),
            Err(err) => {
                self.fail_task(&err.to_string())
                    .await
                    .map_err(|persist_err| StoreError::store(persist_err.to_string()))?;
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

#[async_trait(?Send)]
impl<S> RoleRunner for ParameterScanRunner<S>
where
    S: crate::core::ControlPlaneStore
        + crate::core::AggregationStore
        + crate::core::RunReadStore
        + crate::core::RunSpecStore
        + crate::core::RunTaskStore
        + Send
        + Sync
        + 'static,
{
    async fn tick(&mut self) -> Result<bool, StoreError> {
        ParameterScanRunner::tick(self).await
    }

    async fn stop(&mut self) -> Result<(), StoreError> {
        ParameterScanRunner::stop(self).await
    }

    fn min_tick_time(&self) -> Duration {
        ParameterScanRunner::min_tick_time(self)
    }
}
