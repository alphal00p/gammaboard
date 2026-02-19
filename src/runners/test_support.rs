use crate::batch::{Batch, BatchResults};
use crate::core::{BatchClaim, CompletedBatch, StoreError, WorkQueueStore};
use serde_json::Value as JsonValue;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub(crate) struct MockWorkQueueState {
    pub next_claim: Option<BatchClaim>,
    pub submitted: Vec<(i64, BatchResults, JsonValue)>,
    pub failed: Vec<(i64, String)>,
    pub inserted: Vec<Batch>,
    pub pending_batches: i64,
    pub completed: Vec<CompletedBatch>,
    pub fetch_last_batch_ids: Vec<Option<i64>>,
}

#[derive(Clone, Default)]
pub(crate) struct MockWorkQueue {
    pub inner: Arc<Mutex<MockWorkQueueState>>,
}

#[async_trait::async_trait]
impl WorkQueueStore for MockWorkQueue {
    async fn insert_batch(&self, _run_id: i32, batch: &Batch) -> Result<(), StoreError> {
        let mut guard = self.inner.lock().expect("poison");
        guard.inserted.push(batch.clone());
        guard.pending_batches += 1;
        Ok(())
    }

    async fn get_pending_batch_count(&self, _run_id: i32) -> Result<i64, StoreError> {
        Ok(self.inner.lock().expect("poison").pending_batches)
    }

    async fn claim_batch(
        &self,
        _run_id: i32,
        _worker_id: &str,
    ) -> Result<Option<BatchClaim>, StoreError> {
        Ok(self.inner.lock().expect("poison").next_claim.take())
    }

    async fn submit_batch_results(
        &self,
        batch_id: i64,
        results: &BatchResults,
        batch_observable: &JsonValue,
        _eval_time_ms: f64,
    ) -> Result<(), StoreError> {
        self.inner.lock().expect("poison").submitted.push((
            batch_id,
            results.clone(),
            batch_observable.clone(),
        ));
        Ok(())
    }

    async fn fail_batch(&self, batch_id: i64, last_error: &str) -> Result<(), StoreError> {
        self.inner
            .lock()
            .expect("poison")
            .failed
            .push((batch_id, last_error.to_string()));
        Ok(())
    }

    async fn fetch_completed_batches_since(
        &self,
        _run_id: i32,
        last_batch_id: Option<i64>,
        _limit: usize,
    ) -> Result<Vec<CompletedBatch>, StoreError> {
        let mut guard = self.inner.lock().expect("poison");
        guard.fetch_last_batch_ids.push(last_batch_id);
        Ok(guard.completed.clone())
    }
}
