//! Recording store shared by queue and sampler tests. Unexpected calls fail loudly.
use crate::core::{
    AggregationStore, BatchClaim, BatchQueueCounts, CompletedBatch, ControlPlaneStore,
    DesiredAssignment, EvaluatorPerformanceSnapshot, InsertBatchesMetrics, InsertBatchesOutcome,
    RegisteredNode, RunReadStore, RunSampleProgress, RunStageSnapshot, RunTask, RunTaskInput,
    RunTaskStore, SamplerAggregatorPerformanceSnapshot, StoreError, WorkQueueStore,
};
use crate::sampling::{LatentBatch, LatentBatchPayload};
use crate::stores::{
    EvaluatorPerformanceHistoryEntry, RegisteredWorkerEntry, RunProgress, RuntimeLogPage,
    SamplerPerformanceHistoryEntry, TaskOutputSnapshot, TaskStageSnapshot,
};
use crate::utils::domain::Domain;
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

type RecordedInserts = Arc<Mutex<Vec<(f64, Vec<i64>)>>>;

#[derive(Clone, Default)]
pub(crate) struct RecordingStore {
    inserts: RecordedInserts,
    first_insert_gate: Option<Arc<Notify>>,
    pub(crate) inserted: Arc<Notify>,
    fetch_completed_calls: Arc<Mutex<usize>>,
    pub(crate) completed_ids: Arc<Mutex<Vec<i64>>>,
    pub(crate) work_notifications: Arc<Mutex<usize>>,
}

impl RecordingStore {
    pub(crate) fn block_first_insert(&mut self) -> Arc<Notify> {
        let gate = Arc::new(Notify::new());
        self.first_insert_gate = Some(gate.clone());
        gate
    }

    pub(crate) fn recorded_inserts(&self) -> Vec<(f64, Vec<i64>)> {
        self.inserts.lock().expect("recording lock").clone()
    }

    pub(crate) fn fetch_completed_calls(&self) -> usize {
        *self.fetch_completed_calls.lock().expect("recording lock")
    }
}

#[async_trait]
impl WorkQueueStore for RecordingStore {
    async fn notify_work_available(&self, _run_id: i32) -> Result<(), StoreError> {
        *self.work_notifications.lock().unwrap() += 1;
        Ok(())
    }

    async fn insert_batches(
        &self,
        _run_id: i32,
        _task_id: i64,
        _requires_training_values: bool,
        batch_ids: &[i64],
        batches: &[LatentBatch],
    ) -> Result<InsertBatchesOutcome, StoreError> {
        let logical_weight = match &batches[0].payload {
            LatentBatchPayload::IndexedBatch { weights, .. } => weights[0],
            LatentBatchPayload::HavanaInference { .. }
            | LatentBatchPayload::HavanaInferenceIndexed { .. } => 0.0,
        };
        if logical_weight == 1.0
            && let Some(gate) = &self.first_insert_gate
        {
            gate.notified().await;
        }
        self.inserts
            .lock()
            .expect("recording lock")
            .push((logical_weight, batch_ids.to_vec()));
        self.inserted.notify_one();
        Ok(InsertBatchesOutcome {
            batch_ids: batch_ids.to_vec(),
            metrics: InsertBatchesMetrics::default(),
        })
    }

    async fn get_batch_queue_counts(
        &self,
        _run_id: i32,
        _task_id: Option<i64>,
        _completed_after_batch_id: Option<i64>,
    ) -> Result<BatchQueueCounts, StoreError> {
        unreachable!("unused in test")
    }

    async fn get_queue_blocker(
        &self,
        _run_id: i32,
        _task_id: i64,
        _after_batch_id: Option<i64>,
    ) -> Result<Option<crate::core::QueueBlocker>, StoreError> {
        Ok(None)
    }

    async fn claim_batch(
        &self,
        _run_id: i32,
        _node_uuid: &str,
        _claim_token: &str,
    ) -> Result<Option<BatchClaim>, StoreError> {
        unreachable!("unused in test")
    }

    async fn release_claimed_batches_for_worker(
        &self,
        _run_id: i32,
        _node_uuid: &str,
    ) -> Result<u64, StoreError> {
        unreachable!("unused in test")
    }

    async fn release_untracked_claims(
        &self,
        _run_id: i32,
        _node_uuid: &str,
        _tracked_tokens: &[String],
    ) -> Result<u64, StoreError> {
        unreachable!("unused in test")
    }

    async fn submit_batch_results(
        &self,
        _batch_id: i64,
        _node_uuid: &str,
        _claim_token: &str,
        _result: &crate::evaluation::BatchResult,
        _eval_time_ms: f64,
    ) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }

    async fn record_evaluator_performance_snapshot(
        &self,
        _snapshot: &EvaluatorPerformanceSnapshot,
    ) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }

    async fn record_sampler_performance_snapshot(
        &self,
        _snapshot: &SamplerAggregatorPerformanceSnapshot,
    ) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }

    async fn fail_batch(
        &self,
        _batch_id: i64,
        _node_uuid: &str,
        _claim_token: &str,
        _last_error: &str,
        _max_batch_retries: i32,
    ) -> Result<crate::core::BatchFailOutcome, StoreError> {
        unreachable!("unused in test")
    }

    async fn fetch_completed_batches(
        &self,
        _run_id: i32,
        _task_id: i64,
        _limit: usize,
        _strict_ordering: bool,
        _after_batch_id: Option<i64>,
    ) -> Result<Vec<crate::core::CompletedBatch>, StoreError> {
        *self.fetch_completed_calls.lock().expect("recording lock") += 1;
        Ok(self
            .completed_ids
            .lock()
            .unwrap()
            .iter()
            .copied()
            .filter(|id| *id > _after_batch_id.unwrap_or(0))
            .map(|batch_id| CompletedBatch {
                batch_id,
                task_id: 1,
                requires_training_values: true,
                batch_size: 1,
                result: crate::evaluation::BatchResult::new(
                    None,
                    crate::evaluation::AccumulatorState::Empty(Default::default()),
                ),
                completed_at: None,
                total_eval_time_ms: None,
            })
            .collect())
    }

    async fn cleanup_consumed_completed_batches(
        &self,
        _run_id: i32,
        _up_to_batch_id: i64,
        _limit: usize,
    ) -> Result<u64, StoreError> {
        unreachable!("unused in test")
    }

    async fn reclaim_abandoned_batches(&self, _run_id: i32) -> Result<u64, StoreError> {
        unreachable!("unused in test")
    }
}

#[async_trait]
impl AggregationStore for RecordingStore {
    async fn load_current_accumulator(
        &self,
        _run_id: i32,
    ) -> Result<Option<JsonValue>, StoreError> {
        unreachable!("unused in test")
    }

    async fn persist_task_result_snapshot(
        &self,
        _run_id: i32,
        _task_id: i64,
        _result: &JsonValue,
    ) -> Result<i64, StoreError> {
        unreachable!("unused in test")
    }

    async fn load_sampler_checkpoint(
        &self,
        _run_id: i32,
    ) -> Result<Option<crate::core::SamplerAggregatorCheckpoint>, StoreError> {
        unreachable!("unused in test")
    }

    async fn load_stage_snapshot(
        &self,
        _snapshot_id: i64,
    ) -> Result<Option<RunStageSnapshot>, StoreError> {
        unreachable!("unused in test")
    }

    async fn load_latest_stage_snapshot_before_sequence(
        &self,
        _run_id: i32,
        _sequence_nr: i32,
    ) -> Result<Option<RunStageSnapshot>, StoreError> {
        unreachable!("unused in test")
    }

    async fn load_task_activation_snapshot(
        &self,
        _run_id: i32,
        _task_id: i64,
    ) -> Result<Option<RunStageSnapshot>, StoreError> {
        unreachable!("unused in test")
    }

    async fn load_run_sample_progress(
        &self,
        _run_id: i32,
    ) -> Result<Option<RunSampleProgress>, StoreError> {
        unreachable!("unused in test")
    }

    async fn save_aggregation(
        &self,
        _run_id: i32,
        _task_id: i64,
        _current_accumulator: &JsonValue,
        _persisted_observable: Option<&JsonValue>,
        _delta_batches_completed: i32,
    ) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }

    async fn save_sampler_checkpoint(
        &self,
        _run_id: i32,
        _checkpoint: &crate::core::SamplerAggregatorCheckpoint,
        _stage: Option<&crate::core::RunStageSnapshot>,
    ) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }

    async fn save_run_sample_progress(
        &self,
        _run_id: i32,
        _nr_produced_samples: i64,
        _nr_completed_samples: i64,
        _sampler_runner_uptime_ms: f64,
    ) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }

    async fn save_run_stage_snapshot(
        &self,
        _snapshot: &RunStageSnapshot,
    ) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }
}

#[async_trait]
impl RunTaskStore for RecordingStore {
    async fn append_run_tasks(
        &self,
        _run_id: i32,
        _tasks: &[RunTaskInput],
    ) -> Result<Vec<RunTask>, StoreError> {
        unreachable!("unused in test")
    }

    async fn list_run_tasks(&self, _run_id: i32) -> Result<Vec<RunTask>, StoreError> {
        unreachable!("unused in test")
    }

    async fn load_run_task(&self, _task_id: i64) -> Result<Option<RunTask>, StoreError> {
        unreachable!("unused in test")
    }

    async fn remove_pending_run_task(
        &self,
        _run_id: i32,
        _task_id: i64,
    ) -> Result<bool, StoreError> {
        unreachable!("unused in test")
    }

    async fn update_run_task_queue_tuning(
        &self,
        _run_id: i32,
        _task_id: i64,
        _queue_tuning: Option<crate::core::SamplerQueueTuning>,
    ) -> Result<RunTask, StoreError> {
        unreachable!("unused in test")
    }

    async fn load_active_run_task(&self, _run_id: i32) -> Result<Option<RunTask>, StoreError> {
        unreachable!("unused in test")
    }

    async fn activate_next_run_task(&self, _run_id: i32) -> Result<Option<RunTask>, StoreError> {
        unreachable!("unused in test")
    }

    async fn update_run_task_progress(
        &self,
        _task_id: i64,
        _nr_produced_samples: i64,
        _nr_completed_samples: i64,
    ) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }

    async fn set_run_task_spawn_origin(
        &self,
        _task_id: i64,
        _spawned_from_snapshot_id: Option<i64>,
    ) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }

    async fn complete_run_task(&self, _task_id: i64) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }

    async fn persist_task_measurement_output(
        &self,
        _task_id: i64,
        _output: &crate::core::TaskMeasurementOutput,
    ) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }

    async fn persist_task_controller_output(
        &self,
        _task_id: i64,
        _output: &crate::core::ControllerTaskOutput,
    ) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }

    async fn fail_run_task(&self, _task_id: i64, _reason: &str) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }
}

#[async_trait]
impl ControlPlaneStore for RecordingStore {
    async fn try_lock_task_control(
        &self,
    ) -> Result<Option<Box<dyn Send>>, crate::core::StoreError> {
        Ok(Some(Box::new(())))
    }

    async fn update_desired_assignments(
        &self,
        _updates: &[crate::core::NodeAssignmentUpdate],
    ) -> Result<bool, crate::core::StoreError> {
        unreachable!("unused in test")
    }

    async fn assign_worker_pool(
        &self,
        _node_name: &str,
        _role: crate::core::WorkerRole,
        _run_id: i32,
    ) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }

    async fn announce_node(
        &self,
        _node_name: &str,
        _node_uuid: &str,
        _capabilities: &crate::core::NodeCapabilities,
    ) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }

    async fn set_current_assignment(
        &self,
        _node_uuid: &str,
        _role: crate::core::WorkerRole,
        _run_id: i32,
    ) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }

    async fn clear_current_assignment(&self, _node_uuid: &str) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }

    async fn clear_desired_assignment(&self, _node_name: &str) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }

    async fn clear_desired_assignments_for_run(&self, _run_id: i32) -> Result<u64, StoreError> {
        unreachable!("unused in test")
    }

    async fn clear_desired_assignments_for_run_except_node(
        &self,
        _run_id: i32,
        _keep_node_name: &str,
    ) -> Result<u64, StoreError> {
        unreachable!("unused in test")
    }

    async fn clear_all_desired_assignments(&self) -> Result<u64, StoreError> {
        unreachable!("unused in test")
    }

    async fn get_desired_assignment(
        &self,
        _node_name: &str,
    ) -> Result<Option<DesiredAssignment>, StoreError> {
        unreachable!("unused in test")
    }

    async fn list_desired_assignments(
        &self,
        _node_name: Option<&str>,
    ) -> Result<Vec<DesiredAssignment>, StoreError> {
        unreachable!("unused in test")
    }

    async fn list_nodes(
        &self,
        _node_name: Option<&str>,
    ) -> Result<Vec<RegisteredNode>, StoreError> {
        unreachable!("unused in test")
    }

    async fn create_node_launch_request(
        &self,
        _backend: &str,
        _requested_count: i32,
        _name_prefix: Option<&str>,
        _args: &JsonValue,
    ) -> Result<crate::core::NodeLaunchRequest, StoreError> {
        unreachable!("unused in test")
    }

    async fn list_node_launch_requests(
        &self,
    ) -> Result<Vec<crate::core::NodeLaunchRequest>, StoreError> {
        unreachable!("unused in test")
    }

    async fn claim_external_node_launch_request(
        &self,
    ) -> Result<Option<crate::core::NodeLaunchRequest>, StoreError> {
        unreachable!("unused in test")
    }

    async fn update_node_launch_request_state(
        &self,
        _id: i64,
        _state: &str,
        _started_count: i32,
        _result: &JsonValue,
        _error: Option<&str>,
    ) -> Result<crate::core::NodeLaunchRequest, StoreError> {
        unreachable!("unused in test")
    }

    async fn count_active_evaluator_nodes(&self, _run_id: i32) -> Result<i64, StoreError> {
        Ok(20)
    }

    async fn request_node_shutdown(&self, _node_name: &str) -> Result<u64, StoreError> {
        unreachable!("unused in test")
    }

    async fn request_all_nodes_shutdown(&self) -> Result<u64, StoreError> {
        unreachable!("unused in test")
    }

    async fn consume_node_shutdown_request(&self, _node_uuid: &str) -> Result<bool, StoreError> {
        unreachable!("unused in test")
    }

    async fn expire_node_lease(&self, _node_uuid: &str) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }

    async fn create_run(
        &self,
        _name: &str,
        _run_toml: &str,
        _provenance: &JsonValue,
        _integration_params: &JsonValue,
        _target: Option<&JsonValue>,
        _domain: &Domain,
        _initial_stage_snapshot: &RunStageSnapshot,
        _initial_tasks: &[RunTaskInput],
        _parent: Option<&crate::core::traits::RunParentMetadata>,
    ) -> Result<i32, StoreError> {
        unreachable!("unused in test")
    }

    async fn remove_run(&self, _run_id: i32) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }
}

#[async_trait]
impl RunReadStore for RecordingStore {
    async fn health_check(&self) -> Result<(), StoreError> {
        unreachable!("unused in test")
    }

    async fn get_all_runs(&self) -> Result<Vec<RunProgress>, StoreError> {
        unreachable!("unused in test")
    }

    async fn get_run_progress(&self, _run_id: i32) -> Result<Option<RunProgress>, StoreError> {
        unreachable!("unused in test")
    }

    async fn get_task_output_snapshots(
        &self,
        _run_id: i32,
        _task_id: i64,
        _after_snapshot_id: Option<i64>,
        _limit: i64,
    ) -> Result<Vec<TaskOutputSnapshot>, StoreError> {
        unreachable!("unused in test")
    }

    async fn get_latest_task_stage_snapshot(
        &self,
        _run_id: i32,
        _task_id: i64,
    ) -> Result<Option<TaskStageSnapshot>, StoreError> {
        unreachable!("unused in test")
    }

    async fn get_latest_task_stage_snapshot_id(
        &self,
        _run_id: i32,
        _task_id: i64,
    ) -> Result<Option<String>, StoreError> {
        unreachable!("unused in test")
    }

    async fn get_runtime_logs(
        &self,
        _limit: i64,
        _source: Option<&str>,
        _run_id: Option<i32>,
        _include_child_runs: bool,
        _node_name: Option<&str>,
        _node_uuid: Option<&str>,
        _level: Option<&str>,
        _query: Option<&str>,
        _before_id: Option<i64>,
    ) -> Result<RuntimeLogPage, StoreError> {
        unreachable!("unused in test")
    }

    async fn get_registered_workers(
        &self,
        _run_id: Option<i32>,
    ) -> Result<Vec<RegisteredWorkerEntry>, StoreError> {
        Ok(Vec::new())
    }

    async fn get_evaluator_performance_history(
        &self,
        _run_id: i32,
        _limit: i64,
        _worker_id: Option<&str>,
    ) -> Result<Vec<EvaluatorPerformanceHistoryEntry>, StoreError> {
        unreachable!("unused in test")
    }

    async fn get_sampler_performance_history(
        &self,
        _run_id: i32,
        _limit: i64,
        _worker_id: Option<&str>,
    ) -> Result<Vec<SamplerPerformanceHistoryEntry>, StoreError> {
        unreachable!("unused in test")
    }
}
