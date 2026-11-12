use super::*;
use crate::core::{BatchQueueCounts, RunTaskSpec};
use crate::test_support::RecordingStore;
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
    params.queue.max_batch_size = max_batch;
    params.queue.max_generation_size = max_batch.try_into().unwrap();
    params.queue.max_batches_per_tick = 20;
    let config: SamplingConfig = serde_json::from_value(json!({
        "kind": "naive_monte_carlo", "seed": 42, "training_window_samples": training_window,
    }))
    .unwrap();
    let sampler = config
        .create(Domain::rectangular(2, 0), Some(budget), json!({}))
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
    .unwrap()
}

fn draw_count(runner: &mut SamplerAggregatorRunner<RecordingStore>) -> usize {
    let raw = runner.sampler.snapshot().unwrap();
    raw["produced_batches_total"].as_u64().unwrap() as usize
}

#[tokio::test]
async fn campaign_allowance_bounds_buffered_draws_and_waits_for_another_window() {
    let mut runner = runner(0, 32_768, 40_000);
    *runner.store.campaign_allowance.lock().unwrap() = Some(32);
    runner.produce(BatchQueueCounts::default()).await.unwrap();
    assert_eq!(
        runner.task.nr_produced_samples + runner.runtime_state.generation.pending_samples() as i64,
        32
    );
    runner.runtime_state.accumulator_checkpoint_state = AccumulatorCheckpointState::Ready;
    runner.produce(BatchQueueCounts::default()).await.unwrap();
    assert_eq!(runner.task.nr_produced_samples, 32);
    assert_eq!(
        runner.produce(BatchQueueCounts::default()).await.unwrap().0,
        0
    );
    assert!(runner.waiting_for_campaign_budget);
    assert!(
        !runner
            .check_tick_terminal_state(BatchQueueCounts::default(), 0, 0, true)
            .unwrap()
    );
    *runner.store.campaign_allowance.lock().unwrap() = Some(48);
    runner.produce(BatchQueueCounts::default()).await.unwrap();
    assert_eq!(
        runner.task.nr_produced_samples + runner.runtime_state.generation.pending_samples() as i64,
        48
    );
    runner.queue.flush().await.unwrap();
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

#[tokio::test]
async fn training_draws_survive_soft_queue_overshoot() {
    let mut runner = runner(20_000, 20_000, 40_000);
    runner.runtime_state.accumulator_checkpoint_state = AccumulatorCheckpointState::Ready;
    assert_eq!(
        runner.produce(BatchQueueCounts::default()).await.unwrap(),
        (20, true)
    );
    assert_eq!(draw_count(&mut runner), 1);
    assert_eq!(runner.task.nr_produced_samples, 5000);
    assert_eq!(runner.runtime_state.generation.pending_samples(), 15_000);
    assert!(matches!(
        runner.sampler.generate(20_000).unwrap(),
        Generation::Waiting
    ));

    // A draw already started continues dispatching above the refill threshold.
    for _ in 0..3 {
        assert_eq!(
            runner
                .produce(BatchQueueCounts {
                    pending: 80,
                    ..Default::default()
                })
                .await
                .unwrap(),
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
            .accept_feedback(&crate::sampling::Feedback::new((vec![1.0; 250]).to_vec()))
            .unwrap();
        if batch == 79 {
            let values = result.unwrap();
            assert_eq!(values.len(), 20_000);
            runner.sampler.feedback(&values).unwrap();
        } else {
            assert!(result.is_none());
        }
    }
    assert!(runner.runtime_state.generation.is_empty());
    assert_eq!(runner.sampler.diagnostics()["pending_training_samples"], 0);
    runner.queue.flush().await.unwrap();
}

#[tokio::test]
async fn generation_preserves_initial_probe_and_checkpointed_remainder() {
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
            .accept_feedback(&crate::sampling::Feedback::new(([1.0; 16]).to_vec()))
            .unwrap()
            .is_none()
    );
    runner.runtime_state.accumulator_checkpoint_state = AccumulatorCheckpointState::Ready;
    // Model a pause after the probe has completed: no DB samples outstanding.
    let value = serde_json::to_value(&runner.runtime_state).unwrap();
    runner.runtime_state = serde_json::from_value(value).unwrap();
    assert_eq!(
        runner.produce(BatchQueueCounts::default()).await.unwrap(),
        (20, true)
    );
    assert_eq!(draw_count(&mut runner), 1);
    runner.queue.flush().await.unwrap();
}

#[tokio::test]
async fn draw_respects_queue_limit_and_task_budget_and_works_after_training() {
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

#[tokio::test]
async fn generation_size_is_independent_of_evaluator_limit_and_queue_target() {
    let mut runner = runner(0, 20_000, 40_000);
    runner.params.queue.max_batch_size = 100;
    let mut config = runner.queue.config().clone();
    config.max_batch_size = 100;
    config.fixed_batch_size = Some(100);
    runner.queue.apply_config(config);
    runner.runtime_state.accumulator_checkpoint_state = AccumulatorCheckpointState::Ready;
    for _ in 0..10 {
        assert_eq!(
            runner
                .produce(BatchQueueCounts {
                    pending: if runner.runtime_state.generation.has_pending() {
                        200
                    } else {
                        0
                    },
                    ..Default::default()
                })
                .await
                .unwrap()
                .0,
            20
        );
    }
    assert_eq!(draw_count(&mut runner), 1);
    assert_eq!(runner.task.nr_produced_samples, 20_000);
    assert_eq!(
        runner
            .produce(BatchQueueCounts {
                pending: 200,
                ..Default::default()
            })
            .await
            .unwrap()
            .0,
        0
    );
    assert_eq!(draw_count(&mut runner), 1);
    runner.queue.flush().await.unwrap();
}

#[test]
fn sampler_io_threads_default_to_one_and_reject_zero() {
    let defaults: toml::Value =
        toml::from_str(include_str!("../config_defaults/run.toml")).unwrap();
    let mut params = defaults["sampler_aggregator_runner_params"].clone();
    params.as_table_mut().unwrap().remove("io_threads");
    let defaulted: SamplerAggregatorRunnerParams = params.clone().try_into().unwrap();
    assert_eq!(defaulted.io_threads.get(), 1);
    params
        .as_table_mut()
        .unwrap()
        .insert("io_threads".into(), toml::Value::Integer(3));
    let configured: SamplerAggregatorRunnerParams = params.clone().try_into().unwrap();
    assert_eq!(configured.io_threads.get(), 3);
    params["io_threads"] = toml::Value::Integer(0);
    assert!(params.try_into::<SamplerAggregatorRunnerParams>().is_err());
}

#[tokio::test]
async fn live_generation_limit_preserves_buffered_draw_and_feedback_boundaries() {
    let mut runner = runner(30_000, 20_000, 40_000);
    runner.runtime_state.accumulator_checkpoint_state = AccumulatorCheckpointState::Ready;
    runner.produce(BatchQueueCounts::default()).await.unwrap();
    assert_eq!(
        runner.task.nr_produced_samples as usize
            + runner.runtime_state.generation.pending_samples(),
        20_000
    );

    let mut task = runner.task.clone();
    task.task
        .set_sample_queue_tuning(Some(
            serde_json::from_value(json!({
                "max_generation_size": 8000,
            }))
            .unwrap(),
        ))
        .unwrap();
    *runner.store.active_task.lock().unwrap() = Some(task);
    runner.last_task_config_refresh_at = Instant::now() - TASK_CONFIG_REFRESH_INTERVAL;
    runner.refresh_live_queue_tuning().await.unwrap();
    assert_eq!(runner.params.queue.max_generation_size.get(), 8000);
    assert_eq!(runner.queue.config().max_generation_size.get(), 8000);

    // Finish the already-buffered 20k draw, even after lowering the limit.
    while runner.runtime_state.generation.has_pending() {
        runner.produce(BatchQueueCounts::default()).await.unwrap();
    }
    assert_eq!(draw_count(&mut runner), 1);
    assert_eq!(runner.task.nr_produced_samples, 20_000);
    // New draws are 8k and 2k: the training boundary is still at 30k.
    for expected in [28_000, 30_000] {
        runner.produce(BatchQueueCounts::default()).await.unwrap();
        while runner.runtime_state.generation.has_pending() {
            runner.produce(BatchQueueCounts::default()).await.unwrap();
        }
        assert_eq!(runner.task.nr_produced_samples, expected);
    }
    assert_eq!(draw_count(&mut runner), 3);
    assert!(matches!(
        runner.sampler.generate(8000).unwrap(),
        Generation::Waiting
    ));
    for size in [20_000, 8000, 2000] {
        let values = runner
            .runtime_state
            .generation
            .accept_feedback(&crate::sampling::Feedback::new((vec![1.0; size]).to_vec()))
            .unwrap()
            .unwrap();
        assert_eq!(values.len(), size);
        runner.sampler.feedback(&values).unwrap();
    }
    assert_eq!(runner.sampler.diagnostics()["training_updates"], 1);
    runner.queue.flush().await.unwrap();
}

#[test]
fn generation_limit_defaults_and_validation() {
    let defaults: toml::Value =
        toml::from_str(include_str!("../config_defaults/run.toml")).unwrap();
    let mut queue = defaults["sampler_aggregator_runner_params"]["queue"].clone();
    assert_eq!(
        queue
            .clone()
            .try_into::<SamplerQueueConfig>()
            .unwrap()
            .max_generation_size
            .get(),
        262_144
    );
    queue.as_table_mut().unwrap().remove("max_generation_size");
    assert_eq!(
        queue
            .clone()
            .try_into::<SamplerQueueConfig>()
            .unwrap()
            .max_generation_size
            .get(),
        262_144
    );
    queue
        .as_table_mut()
        .unwrap()
        .insert("max_generation_size".into(), toml::Value::Integer(0));
    assert!(queue.try_into::<SamplerQueueConfig>().is_err());
    assert!(
        serde_json::from_value::<SamplerQueueTuning>(json!({"max_generation_size": 0})).is_err()
    );
}

#[test]
fn periodic_checkpoint_defaults_and_task_local_thresholds() {
    let mut run = runner(0, 100, 1000);
    let policy = run.task.task.checkpoint_policy().unwrap();
    assert_eq!(policy.interval_seconds, 3600.0);
    assert_eq!(policy.interval_samples, 0);
    run.runtime_state.sampler_uptime_ms_accumulated = 100.0 * 3600.0 * 1000.0;
    run.task.nr_completed_samples = 900;
    assert!(
        !run.periodic_checkpoint_due(),
        "prior uptime or sample volume cannot trigger the default time-only policy"
    );
    run.last_checkpoint_at = Instant::now() - Duration::from_secs(3601);
    assert!(run.periodic_checkpoint_due());
    if let RunTaskSpec::Sample { checkpoint, .. } = &mut run.task.task {
        checkpoint.interval_seconds = 0.0;
        checkpoint.interval_samples = 100;
    }
    run.last_checkpoint_samples = 850;
    assert!(!run.periodic_checkpoint_due());
    run.task.nr_completed_samples = 950;
    assert!(run.periodic_checkpoint_due());
    run.last_checkpoint_samples = 950;
    assert!(
        !run.periodic_checkpoint_due(),
        "threshold counts newly completed task samples"
    );
    if let RunTaskSpec::Sample { checkpoint, .. } = &mut run.task.task {
        checkpoint.interval_samples = 0;
    }
    assert!(
        !run.periodic_checkpoint_due(),
        "both triggers can be disabled"
    );
}

#[test]
fn periodic_checkpoint_config_roundtrips_and_rejects_invalid_thresholds() {
    let mut task = runner(0, 100, 1000).task.task;
    for value in [
        json!({}),
        json!({"interval_seconds":0.25,"interval_samples":7}),
        json!({"interval_seconds":0,"interval_samples":0}),
    ] {
        let mut encoded = serde_json::to_value(&task).unwrap();
        encoded["checkpoint"] = value;
        task = serde_json::from_value(encoded).unwrap();
        task.validate().unwrap();
        let input = crate::core::RunTaskInput {
            name: Some("sample".into()),
            task: task.clone(),
        };
        let text = crate::core::canonical_task_toml(&input).unwrap();
        let document: toml::Value = toml::from_str(&text).unwrap();
        let restored: crate::core::RunTaskInput = document["task"].clone().try_into().unwrap();
        assert_eq!(restored.task.checkpoint_policy(), task.checkpoint_policy());
    }
    for seconds in [-1.0, f64::NAN, f64::INFINITY, f64::MAX] {
        assert!(
            crate::core::CheckpointPolicy {
                interval_seconds: seconds,
                interval_samples: 0
            }
            .validate()
            .is_err()
        );
    }
    assert!(
        crate::core::CheckpointPolicy {
            interval_seconds: 3600.,
            interval_samples: u64::MAX
        }
        .validate()
        .is_err()
    );
}

#[tokio::test]
async fn periodic_checkpoint_flushes_pending_draws_and_does_not_create_a_stage() {
    let mut run = runner(20_000, 20_000, 40_000);
    *run.store.checkpoints.lock().unwrap() = Some(Default::default());
    run.save_initial_checkpoint().await.unwrap();
    run.produce(BatchQueueCounts::default()).await.unwrap();
    assert!(run.runtime_state.generation.has_pending());
    let pending = run.runtime_state.generation.pending_samples();
    run.last_checkpoint_at = Instant::now() - Duration::from_secs(3601);
    run.maybe_checkpoint().await.unwrap();
    assert!(!run.periodic_checkpoint_due());
    let guard = run.store.checkpoints.lock().unwrap();
    let recorded = guard.as_ref().unwrap();
    assert_eq!(recorded.saved.len(), 2);
    let (saved, stage) = recorded.saved.last().unwrap();
    assert!(
        stage.is_none(),
        "periodic checkpoints replace active recovery state, not named stages"
    );
    assert_eq!(saved.runtime_state.generation.pending_samples(), pending);
    assert_eq!(saved.produced_samples(), run.task.nr_produced_samples);
    assert_eq!(saved.completed_samples, 0);
    assert!(
        !run.store.recorded_inserts().is_empty(),
        "pending insert work was committed before saving"
    );
    let checkpoint = recorded
        .events
        .iter()
        .rposition(|e| *e == "checkpoint")
        .unwrap();
    for step in [
        "aggregation",
        "performance",
        "task_progress",
        "run_progress",
    ] {
        assert!(
            recorded.events[..checkpoint].contains(&step),
            "{step} precedes the recovery boundary"
        );
    }
}

#[tokio::test]
async fn failed_periodic_checkpoint_keeps_the_last_boundary_and_retries_with_backoff() {
    let mut run = runner(0, 100, 1000);
    *run.store.checkpoints.lock().unwrap() = Some(Default::default());
    run.save_initial_checkpoint().await.unwrap();
    run.last_checkpoint_at = Instant::now() - Duration::from_secs(3601);
    let previous = run.last_checkpoint_at;
    run.store
        .checkpoints
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .errors
        .push_back(StoreError::RetryableTransaction {
            sqlstate: "55P03".into(),
            message: "lock timeout".into(),
        });
    run.maybe_checkpoint().await.unwrap();
    assert_eq!(run.last_checkpoint_at, previous);
    assert!(run.periodic_checkpoint_due());
    run.maybe_checkpoint().await.unwrap();
    assert_eq!(
        run.store
            .checkpoints
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .saved
            .len(),
        1
    );
    run.last_checkpoint_attempt_at = Some(Instant::now() - CHECKPOINT_RETRY_INTERVAL);
    run.maybe_checkpoint().await.unwrap();
    assert!(!run.periodic_checkpoint_due());
    assert_eq!(
        run.store
            .checkpoints
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .saved
            .len(),
        2
    );
}
