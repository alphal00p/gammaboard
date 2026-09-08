use super::{PgStore, queries};
use crate::core::{RuntimeLogEvent, RuntimeLogStore, StoreError};

#[async_trait::async_trait]
impl RuntimeLogStore for PgStore {
    async fn insert_runtime_log(&self, event: &RuntimeLogEvent) -> Result<(), StoreError> {
        queries::insert_runtime_log(&self.pool, event).await?;
        Ok(())
    }
}
