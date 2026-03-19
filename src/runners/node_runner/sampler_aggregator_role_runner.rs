use crate::core::StoreError;
use crate::runners::sampler_aggregator::SamplerAggregatorRunner;
use std::time::Duration;
use tokio::{sync::watch, time::sleep};
use tracing::Instrument;
use tracing::{info, warn};

use super::{NodeRunnerStore, active_worker::ActiveWorker};

pub(crate) async fn run_sampler_aggregator_role<S: NodeRunnerStore>(
    worker: &ActiveWorker<S>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<(), StoreError> {
    let Some(spec) = worker.store.load_run_spec(worker.run_id).await? else {
        warn!("run has no RunSpec; sampler-aggregator not started");
        return Ok(());
    };
    let saved_snapshot = worker
        .store
        .load_sampler_runner_snapshot(worker.run_id)
        .await?;

    let engine_span = tracing::span!(tracing::Level::TRACE, "sampler_engine_context");
    let initial_task = match worker.store.load_active_run_task(worker.run_id).await? {
        Some(task) => Some(task),
        None => worker
            .store
            .list_run_tasks(worker.run_id)
            .await?
            .into_iter()
            .find(|task| {
                !matches!(task.task, crate::core::RunTaskSpec::Pause)
                    && matches!(
                        task.state,
                        crate::core::RunTaskState::Pending | crate::core::RunTaskState::Active
                    )
            }),
    };
    let initial_sample_budget = initial_task.and_then(|task| {
        task.task
            .nr_expected_samples()
            .and_then(|n| usize::try_from(n).ok())
    });
    let engine = {
        let _engine_scope = engine_span.enter();
        let engine = if let Some(snapshot) = saved_snapshot.as_ref() {
            snapshot
                .engine
                .clone()
                .into_runtime(&spec.point_spec)
                .map_err(|err| {
                    StoreError::store(format!(
                        "failed to restore sampler-aggregator from snapshot: {err}"
                    ))
                })?
        } else {
            spec.sampler_aggregator
                .build(spec.point_spec.clone(), initial_sample_budget, None)
                .map_err(|err| {
                    StoreError::store(format!("failed to build sampler-aggregator: {err}"))
                })?
        };
        engine
            .validate_point_spec(&spec.point_spec)
            .map_err(|err| {
                StoreError::store(format!(
                    "incompatible sampler-aggregator for point_spec on run {}: {}",
                    worker.run_id, err
                ))
            })?;
        engine
    };
    let observable_state = spec
        .evaluator
        .empty_observable_state(&spec.observable)
        .map_err(|err| {
            StoreError::store(format!("failed to initialize observable state: {err}"))
        })?;

    let _ = spec.sampler_aggregator.kind_str();
    worker.mark_active_with_log().await?;

    info!("sampler-aggregator worker started");

    let mut runner = SamplerAggregatorRunner::new(
        worker.run_id,
        worker.node_id.clone(),
        engine,
        observable_state,
        worker.store.clone(),
        worker.store.clone(),
        worker.store.clone(),
        worker.store.clone(),
        spec.sampler_aggregator_runner_params.clone(),
        spec.point_spec.clone(),
        spec.evaluator.clone(),
        spec.sampler_aggregator.clone(),
        spec.parametrization.clone(),
    )
    .await
    .map_err(|err| StoreError::store(err.to_string()))?;

    if let Some(snapshot) = saved_snapshot {
        runner.restore_snapshot(snapshot).map_err(|err| {
            StoreError::store(format!("failed to restore sampler runner snapshot: {err}"))
        })?;
    }

    let interval = Duration::from_millis(spec.sampler_aggregator_runner_params.min_poll_time_ms);

    loop {
        if *stop_rx.borrow() {
            break;
        }

        worker.heartbeat_with_log().await;

        match runner.tick().instrument(engine_span.clone()).await {
            Ok(_) => {}
            Err(err) => warn!("sampler-aggregator tick failed: {err}"),
        }

        tokio::select! {
            _ = stop_rx.changed() => {}
            _ = sleep(interval) => {}
        }
    }

    if let Err(err) = runner.persist_snapshot().await {
        warn!("failed to persist sampler-aggregator snapshot on shutdown: {err}");
    }

    worker.mark_inactive_with_log().await;
    info!("sampler-aggregator worker stopped");

    Ok(())
}
