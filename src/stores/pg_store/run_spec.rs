use super::{PgStore, decode_run_spec, queries};
use crate::core::{RunSpec, RunSpecStore, StoreError};

#[async_trait::async_trait]
impl RunSpecStore for PgStore {
    async fn load_run_spec(&self, run_id: i32) -> Result<Option<RunSpec>, StoreError> {
        let Some((integration_params, domain)) =
            queries::load_run_spec_payload(&self.pool, run_id).await?
        else {
            return Ok(None);
        };
        Ok(Some(decode_run_spec(run_id, integration_params, domain)?))
    }
}
