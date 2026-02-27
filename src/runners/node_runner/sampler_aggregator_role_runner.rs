use crate::core::{StoreError, WorkerRole};
use crate::engines::SamplerAggregatorFactory;
use crate::engines::observable::ObservableFactory;
use crate::runners::sampler_aggregator::SamplerAggregatorRunner;
use std::time::Duration;
use tokio::{sync::watch, time::sleep};
use tracing::{info, warn};

use super::{NodeRunnerStore, active_worker::ActiveWorker};

pub(crate) async fn run_sampler_aggregator_role<S: NodeRunnerStore>(
    worker: &ActiveWorker<S>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<(), StoreError> {
    let Some(spec) = worker.store.load_run_spec(worker.run_id).await? else {
        warn!(
            target: "worker_log",
            run_id = worker.run_id,
            node_id = %worker.node_id,
            worker_id = %worker.worker_id,
            role = %WorkerRole::SamplerAggregator,
            event_type = "run_spec_missing",
            "run has no RunSpec; sampler-aggregator not started"
        );
        return Ok(());
    };

    let engine = SamplerAggregatorFactory::new(
        spec.sampler_aggregator_implementation,
        spec.sampler_aggregator_params.clone(),
    )
    .build()
    .map_err(|err| StoreError::store(format!("failed to build sampler-aggregator: {err}")))?;
    engine
        .validate_point_spec(&spec.point_spec)
        .map_err(|err| {
            StoreError::store(format!(
                "incompatible sampler-aggregator for point_spec on run {}: {}",
                worker.run_id, err
            ))
        })?;
    let mut engine = engine;
    let sampler_init_metadata = engine.get_init_metadata();
    if worker
        .store
        .try_set_sampler_init_metadata(worker.run_id, &sampler_init_metadata)
        .await?
    {
        info!(
            target: "worker_log",
            run_id = worker.run_id,
            node_id = %worker.node_id,
            worker_id = %worker.worker_id,
            role = %WorkerRole::SamplerAggregator,
            event_type = "init_metadata_written",
            "stored sampler-aggregator init metadata"
        );
    }

    let observable_factory =
        ObservableFactory::new(spec.observable_implementation, spec.observable_params);

    worker
        .register_active_worker(spec.sampler_aggregator_implementation.as_ref())
        .await?;

    info!(
        target: "worker_log",
        run_id = worker.run_id,
        node_id = %worker.node_id,
        worker_id = %worker.worker_id,
        role = %WorkerRole::SamplerAggregator,
        event_type = "worker_started",
        "sampler-aggregator worker started"
    );

    let mut runner = SamplerAggregatorRunner::new(
        worker.run_id,
        worker.worker_id.clone(),
        engine,
        observable_factory,
        worker.store.clone(),
        worker.store.clone(),
        spec.sampler_aggregator_runner_params.clone(),
        spec.point_spec.clone(),
    )
    .await
    .map_err(|err| StoreError::store(err.to_string()))?;

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
                warn!(
                    target: "worker_log",
                    run_id = worker.run_id,
                    node_id = %worker.node_id,
                    worker_id = %worker.worker_id,
                    role = %WorkerRole::SamplerAggregator,
                    event_type = "lease_operation_failed",
                    error = %err,
                    "lease operation failed"
                );
                owns_lease = false;
            }
        }

        if owns_lease {
            match runner.tick().await {
                Ok(tick) => {
                    if tick.queue_depleted {
                        info!(
                            target: "worker_log",
                            run_id = worker.run_id,
                            node_id = %worker.node_id,
                            worker_id = %worker.worker_id,
                            role = %WorkerRole::SamplerAggregator,
                            event_type = "queue_depleted",
                            "sampler queue depleted (pending == 0 before tick)"
                        );
                    }
                }
                Err(err) => warn!(
                    target: "worker_log",
                    run_id = worker.run_id,
                    node_id = %worker.node_id,
                    worker_id = %worker.worker_id,
                    role = %WorkerRole::SamplerAggregator,
                    event_type = "tick_failed",
                    error = %err,
                    "sampler-aggregator tick failed"
                ),
            }
        }

        tokio::select! {
            _ = stop_rx.changed() => {}
            _ = sleep(interval) => {}
        }
    }

    if owns_lease
        && let Err(err) = worker
            .store
            .release_sampler_aggregator_lease(worker.run_id, &worker.worker_id)
            .await
    {
        warn!(
            target: "worker_log",
            run_id = worker.run_id,
            node_id = %worker.node_id,
            worker_id = %worker.worker_id,
            role = %WorkerRole::SamplerAggregator,
            event_type = "lease_release_failed",
            error = %err,
            "failed to release sampler-aggregator lease"
        );
    }

    worker.mark_inactive_with_log().await;
    info!(
        target: "worker_log",
        run_id = worker.run_id,
        node_id = %worker.node_id,
        worker_id = %worker.worker_id,
        role = %WorkerRole::SamplerAggregator,
        event_type = "worker_stopped",
        "sampler-aggregator worker stopped"
    );

    Ok(())
}
