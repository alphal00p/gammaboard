use crate::batch::{Batch, BatchResult};
use crate::core::{
    BatchClaim, CompletedBatch, EvaluatorPerformanceSnapshot, SamplerAggregatorPerformanceSnapshot,
    StoreError, WorkQueueStore,
};
use serde_json::Value as JsonValue;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub(crate) struct MockWorkQueueState {
    pub next_claim: Option<BatchClaim>,
    pub submitted: Vec<(i64, BatchResult, JsonValue)>,
    pub evaluator_perf_snapshots: Vec<EvaluatorPerformanceSnapshot>,
    pub sampler_perf_snapshots: Vec<SamplerAggregatorPerformanceSnapshot>,
    pub failed: Vec<(i64, String)>,
    pub next_insert_batch_id: i64,
    pub inserted: Vec<Batch>,
    pub inserted_batch_ids: Vec<i64>,
    pub pending_batches: i64,
    pub completed: Vec<CompletedBatch>,
    pub deleted_completed_batch_ids: Vec<i64>,
}

#[derive(Clone, Default)]
pub(crate) struct MockWorkQueue {
    pub inner: Arc<Mutex<MockWorkQueueState>>,
}

#[async_trait::async_trait]
impl WorkQueueStore for MockWorkQueue {
    async fn insert_batch(&self, _run_id: i32, batch: &Batch) -> Result<i64, StoreError> {
        let mut guard = self.inner.lock().expect("poison");
        guard.next_insert_batch_id += 1;
        let batch_id = guard.next_insert_batch_id;
        guard.inserted.push(batch.clone());
        guard.inserted_batch_ids.push(batch_id);
        guard.pending_batches += 1;
        Ok(batch_id)
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
        result: &BatchResult,
        _eval_time_ms: f64,
    ) -> Result<(), StoreError> {
        self.inner.lock().expect("poison").submitted.push((
            batch_id,
            result.clone(),
            result.observable.clone(),
        ));
        Ok(())
    }

    async fn record_evaluator_performance_snapshot(
        &self,
        snapshot: &EvaluatorPerformanceSnapshot,
    ) -> Result<(), StoreError> {
        self.inner
            .lock()
            .expect("poison")
            .evaluator_perf_snapshots
            .push(snapshot.clone());
        Ok(())
    }

    async fn record_sampler_performance_snapshot(
        &self,
        snapshot: &SamplerAggregatorPerformanceSnapshot,
    ) -> Result<(), StoreError> {
        self.inner
            .lock()
            .expect("poison")
            .sampler_perf_snapshots
            .push(snapshot.clone());
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

    async fn fetch_completed_batches(
        &self,
        _run_id: i32,
        _limit: usize,
    ) -> Result<Vec<CompletedBatch>, StoreError> {
        Ok(self.inner.lock().expect("poison").completed.clone())
    }

    async fn delete_completed_batches(&self, batch_ids: &[i64]) -> Result<(), StoreError> {
        let mut guard = self.inner.lock().expect("poison");
        guard.deleted_completed_batch_ids.extend(batch_ids);
        guard
            .completed
            .retain(|batch| !batch_ids.contains(&batch.batch_id));
        Ok(())
    }
}
