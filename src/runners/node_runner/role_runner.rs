use crate::core::StoreError;
use crate::runners::{EvaluatorRunner, SamplerAggregatorRunner};
use async_trait::async_trait;
use std::time::Duration;

use super::NodeRunnerStore;

#[async_trait(?Send)]
pub(super) trait RoleRunner {
    async fn tick(&mut self) -> Result<bool, StoreError>;
    async fn stop(&mut self) -> Result<(), StoreError>;
    fn min_tick_time(&self) -> Duration;
}

#[async_trait(?Send)]
impl<S: NodeRunnerStore> RoleRunner for EvaluatorRunner<S> {
    async fn tick(&mut self) -> Result<bool, StoreError> {
        EvaluatorRunner::tick(self)
            .await
            .map(|_| false)
            .map_err(|err| StoreError::store(err.to_string()))
    }

    async fn stop(&mut self) -> Result<(), StoreError> {
        EvaluatorRunner::stop(self)
            .await
            .map_err(|err| StoreError::store(err.to_string()))
    }

    fn min_tick_time(&self) -> Duration {
        Duration::from_millis(self.params().min_tick_time_ms)
    }
}

#[async_trait(?Send)]
impl<S: NodeRunnerStore> RoleRunner for SamplerAggregatorRunner<S> {
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
            Err(err) => {
                self.fail_task(&err.to_string())
                    .await
                    .map_err(|persist_err| StoreError::store(persist_err.to_string()))?;
                Err(StoreError::store(err.to_string()))
            }
        }
    }

    async fn stop(&mut self) -> Result<(), StoreError> {
        SamplerAggregatorRunner::persist_state(self)
            .await
            .map_err(|err| StoreError::store(err.to_string()))
    }

    fn min_tick_time(&self) -> Duration {
        Duration::from_millis(self.params().min_tick_time_ms)
    }
}
