use crate::core::StoreError;
use crate::runners::sampler_aggregator::{
    SamplerAggregatorRunner, SamplerAggregatorRunnerSnapshot,
};
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
        .await?
        .map(|payload| {
            serde_json::from_value::<SamplerAggregatorRunnerSnapshot>(payload).map_err(|err| {
                StoreError::store(format!("failed to decode sampler snapshot: {err}"))
            })
        })
        .transpose()?;

    let engine_span = tracing::span!(tracing::Level::TRACE, "sampler_engine_context");
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
                .build(spec.point_spec.clone())
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
    let aggregated_observable = spec.evaluator.empty_observable_state().map_err(|err| {
        StoreError::store(format!("failed to initialize observable state: {err}"))
    })?;

    worker
        .register_active_worker(spec.sampler_aggregator.kind_str())
        .await?;

    info!("sampler-aggregator worker started");

    let mut runner = SamplerAggregatorRunner::new(
        worker.run_id,
        worker.worker_id.clone(),
        engine,
        aggregated_observable,
        worker.store.clone(),
        worker.store.clone(),
        worker.store.clone(),
        spec.sampler_aggregator_runner_params.clone(),
        spec.point_spec.clone(),
    )
    .await
    .map_err(|err| StoreError::store(err.to_string()))?;

    if let Some(snapshot) = saved_snapshot {
        runner.restore_snapshot(snapshot).map_err(|err| {
            StoreError::store(format!("failed to restore sampler runner snapshot: {err}"))
        })?;
    }

    let lease_ttl = Duration::from_millis(spec.sampler_aggregator_runner_params.lease_ttl_ms);
    let interval = Duration::from_millis(spec.sampler_aggregator_runner_params.min_poll_time_ms);
    let mut owns_lease = false;

    loop {
        if *stop_rx.borrow() {
            break;
        }

        worker.heartbeat_with_log().await;

        let lease_result = if owns_lease {
            worker
                .store
                .renew_sampler_aggregator_lease(worker.run_id, &worker.worker_id, lease_ttl)
                .await
        } else {
            worker
                .store
                .acquire_sampler_aggregator_lease(worker.run_id, &worker.worker_id, lease_ttl)
                .await
        };

        match lease_result {
            Ok(has_lease) => owns_lease = has_lease,
            Err(err) => {
                warn!("lease operation failed: {err}");
                owns_lease = false;
            }
        }

        if owns_lease {
            match runner.tick().instrument(engine_span.clone()).await {
                Ok(_) => {}
                Err(err) => warn!("sampler-aggregator tick failed: {err}"),
            }
        }

        tokio::select! {
            _ = stop_rx.changed() => {}
            _ = sleep(interval) => {}
        }
    }

    if let Err(err) = runner.persist_snapshot().await {
        warn!("failed to persist sampler-aggregator snapshot on shutdown: {err}");
    }

    if owns_lease
        && let Err(err) = worker
            .store
            .release_sampler_aggregator_lease(worker.run_id, &worker.worker_id)
            .await
    {
        warn!("failed to release sampler-aggregator lease: {err}");
    }

    worker.mark_inactive_with_log().await;
    info!("sampler-aggregator worker stopped");

    Ok(())
}
