use super::*;
use crate::core::BatchQueueCounts;
use crate::runners::queue::tests::RecordingStore;
use crate::utils::domain::Domain;

fn runner(
    training_window: usize,
    max_batch: usize,
    budget: usize,
) -> SamplerAggregatorRunner<RecordingStore> {
    let defaults: toml::Value =
        toml::from_str(include_str!("../config_defaults/run.toml")).unwrap();
    let mut params: SamplerAggregatorRunnerParams = defaults["sampler_aggregator_runner_params"]
        .clone()
        .try_into()
        .unwrap();
    params.queue.bulk_sample_generation = true;
    params.queue.max_batch_size = max_batch;
    params.queue.queue_buffer = 1.0;
    params.queue.max_batches_per_tick = 20;
    let config: SamplerAggregatorConfig = serde_json::from_value(json!({
        "kind": "naive_monte_carlo", "seed": 42, "training_window_samples": training_window,
    }))
    .unwrap();
    let sampler = config
        .build(Domain::rectangular(2, 0), Some(budget), None, json!({}))
        .unwrap();
    let task = serde_json::from_value(json!({
        "id": 1, "run_id": 1, "name": "test", "sequence_nr": 1,
        "state": "active", "nr_produced_samples": 0, "nr_completed_samples": 0,
        "created_at": "2026-09-23T00:00:00Z", "task_toml": "",
        "task": { "kind": "sample", "stop_condition": {"max_samples": budget} },
    }))
    .unwrap();
    SamplerAggregatorRunner::new(
        RecordingStore::default(),
        1,
        "sampler",
        "sampler-uuid",
        task,
        sampler,
        AccumulatorState::empty_scalar(),
        serde_json::from_value(json!({"kind":"unit", "continuous_dims":2, "discrete_dims":0}))
            .unwrap(),
        config,
        vec![],
        params.clone(),
        params.queue,
        1000,
        RunSampleProgress::default(),
        None,
    )
}

fn draw_count(runner: &mut SamplerAggregatorRunner<RecordingStore>) -> usize {
    let SamplerAggregatorSnapshot::NaiveMonteCarlo { raw } = runner.sampler.snapshot().unwrap()
    else {
        panic!()
    };
    raw["produced_batches_total"].as_u64().unwrap() as usize
}

#[tokio::test]
async fn persistence_reports_execution_time_returned_by_the_operation() {
    let mut runner = runner(100, 100, 100);
    runner
        .consume_aggregation_flush_task(PendingAggregationFlushTask {
            flushed_completed_batches: 0,
            cleared_initial_round_trip: false,
            handle: tokio::spawn(async { Ok(Duration::from_millis(7)) }),
        })
        .await
        .unwrap();
    assert_eq!(
        runner.window_state.persist_accumulator_ms.snapshot().total,
        Some(7.0)
    );
}

use crate::sampling::SamplerAggregatorSnapshot;

#[tokio::test]
async fn bulk_training_draws_one_window_and_dispatches_under_queue_limits() {
    let mut runner = runner(20_000, 20_000, 40_000);
    runner.runtime_state.accumulator_checkpoint_state = AccumulatorCheckpointState::Ready;
    assert_eq!(
        runner.produce(BatchQueueCounts::default()).await.unwrap(),
        (20, true)
    );
    assert_eq!(draw_count(&mut runner), 1);
    assert_eq!(runner.task.nr_produced_samples, 5000);
    assert_eq!(runner.runtime_state.generation.pending_samples(), 15_000);
    assert_eq!(runner.sampler.training_samples_remaining(), Some(0));

    // Backpressure leaves the generated remainder untouched, even after a live toggle.
    runner.params.queue.bulk_sample_generation = false;
    assert_eq!(
        runner
            .produce(BatchQueueCounts {
                pending: 20,
                ..Default::default()
            })
            .await
            .unwrap(),
        (0, true)
    );
    for _ in 0..3 {
        assert_eq!(
            runner.produce(BatchQueueCounts::default()).await.unwrap(),
            (20, true)
        );
    }
    assert_eq!(draw_count(&mut runner), 1);
    assert_eq!(runner.task.nr_produced_samples, 20_000);
    assert!(!runner.runtime_state.generation.has_pending());
    for batch in 0..80 {
        let result = runner
            .runtime_state
            .generation
            .accept_training_values(&vec![1.0; 250])
            .unwrap();
        if batch == 79 {
            let values = result.unwrap();
            assert_eq!(values.len(), 20_000);
            runner.sampler.ingest_training_values(&values).unwrap();
        } else {
            assert!(result.is_none());
        }
    }
    assert!(runner.runtime_state.generation.is_empty());
    assert_eq!(runner.sampler.training_samples_remaining(), Some(20_000));
    runner.queue.flush().await.unwrap();
}

#[tokio::test]
async fn bulk_generation_preserves_initial_probe_and_checkpointed_remainder() {
    let mut runner = runner(20_000, 20_000, 40_000);
    assert_eq!(
        runner.produce(BatchQueueCounts::default()).await.unwrap(),
        (1, true)
    );
    assert_eq!(draw_count(&mut runner), 1);
    assert_eq!(runner.task.nr_produced_samples, 16);
    assert_eq!(runner.runtime_state.generation.pending_samples(), 19_984);
    assert_eq!(
        runner.produce(BatchQueueCounts::default()).await.unwrap(),
        (0, true)
    );
    assert!(
        runner
            .runtime_state
            .generation
            .accept_training_values(&[1.0; 16])
            .unwrap()
            .is_none()
    );
    runner.runtime_state.accumulator_checkpoint_state = AccumulatorCheckpointState::Ready;
    // Model a pause after the probe has completed: no DB samples outstanding.
    let value = serde_json::to_value(&runner.runtime_state).unwrap();
    runner.runtime_state = serde_json::from_value(value).unwrap();
    runner.runtime_state.generation.restore_legacy_samples(0);
    assert_eq!(
        runner.produce(BatchQueueCounts::default()).await.unwrap(),
        (20, true)
    );
    assert_eq!(draw_count(&mut runner), 1);
    runner.queue.flush().await.unwrap();
}

#[tokio::test]
async fn bulk_draw_respects_maximum_and_task_budget_and_works_after_training() {
    for (window, maximum, budget, expected) in [
        (20_000, 8000, 40_000, 8000),
        (20_000, 20_000, 777, 777),
        (0, 20_000, 40_000, 20_000),
    ] {
        let mut runner = runner(window, maximum, budget);
        runner.runtime_state.accumulator_checkpoint_state = AccumulatorCheckpointState::Ready;
        runner.produce(BatchQueueCounts::default()).await.unwrap();
        assert_eq!(draw_count(&mut runner), 1);
        assert_eq!(
            runner.task.nr_produced_samples as usize
                + runner.runtime_state.generation.pending_samples(),
            expected
        );
        runner.queue.flush().await.unwrap();
    }
}

#[tokio::test]
async fn legacy_mode_keeps_one_generation_call_per_evaluator_batch() {
    let mut runner = runner(20_000, 20_000, 40_000);
    runner.params.queue.bulk_sample_generation = false;
    runner.runtime_state.accumulator_checkpoint_state = AccumulatorCheckpointState::Ready;
    assert_eq!(
        runner.produce(BatchQueueCounts::default()).await.unwrap(),
        (20, true)
    );
    assert_eq!(draw_count(&mut runner), 20);
    assert!(!runner.runtime_state.generation.has_pending());
    assert_eq!(runner.task.nr_produced_samples, 5000);
    runner.queue.flush().await.unwrap();
}

#[tokio::test]
async fn inference_chunks_adapt_after_probe_without_another_generation_call() {
    let mut runner = runner(0, 20_000, 40_000);
    assert_eq!(
        runner.produce(BatchQueueCounts::default()).await.unwrap(),
        (1, true)
    );
    runner.runtime_state.accumulator_checkpoint_state = AccumulatorCheckpointState::Ready;
    runner.queue.observe_completed_eval_batch(16, 160.0);
    assert_eq!(runner.queue.current_batch_size(), 200);
    assert_eq!(
        runner.produce(BatchQueueCounts::default()).await.unwrap(),
        (20, true)
    );
    assert_eq!(runner.task.nr_produced_samples, 4016);
    assert_eq!(runner.runtime_state.generation.pending_samples(), 15_984);
    assert_eq!(draw_count(&mut runner), 1);
    runner.queue.flush().await.unwrap();
}
