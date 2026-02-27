use crate::core::{StoreError, WorkerRole};
use crate::engines::observable::ObservableFactory;
use crate::engines::{EvaluatorFactory, ParametrizationFactory};
use crate::runners::evaluator::EvaluatorRunner;
use std::time::Duration;
use tokio::{sync::watch, time::sleep};
use tracing::{info, warn};

use super::{NodeRunnerStore, active_worker::ActiveWorker};

pub(crate) async fn run_evaluator_role<S: NodeRunnerStore>(
    worker: &ActiveWorker<S>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<(), StoreError> {
    let Some(spec) = worker.store.load_run_spec(worker.run_id).await? else {
        warn!(
            target: "worker_log",
            run_id = worker.run_id,
            node_id = %worker.node_id,
            worker_id = %worker.worker_id,
            role = %WorkerRole::Evaluator,
            event_type = "run_spec_missing",
            "run has no RunSpec; evaluator not started"
        );
        return Ok(());
    };

    let evaluator_factory =
        EvaluatorFactory::new(spec.evaluator_implementation, spec.evaluator_params.clone());
    let evaluator = evaluator_factory
        .build()
        .map_err(|err| StoreError::store(format!("failed to build evaluator: {err}")))?;
    evaluator
        .validate_point_spec(&spec.point_spec)
        .map_err(|err| {
            StoreError::store(format!(
                "incompatible evaluator for point_spec on run {}: {}",
                worker.run_id, err
            ))
        })?;

    let observable_factory = ObservableFactory::new(
        spec.observable_implementation,
        spec.observable_params.clone(),
    );
    observable_factory
        .build()
        .map_err(|err| StoreError::store(format!("failed to build observable: {err}")))?;

    if !evaluator.supports_observable(&observable_factory) {
        return Err(StoreError::store(format!(
            "incompatible evaluator/observable pair for run {}: evaluator={} observable={}",
            worker.run_id, spec.evaluator_implementation, spec.observable_implementation
        )));
    }

    let evaluator_init_metadata = evaluator.get_init_metadata();
    if worker
        .store
        .try_set_evaluator_init_metadata(worker.run_id, &evaluator_init_metadata)
        .await?
    {
        info!(
            target: "worker_log",
            run_id = worker.run_id,
            node_id = %worker.node_id,
            worker_id = %worker.worker_id,
            role = %WorkerRole::Evaluator,
            event_type = "init_metadata_written",
            "stored evaluator init metadata"
        );
    }

    let parametrization = ParametrizationFactory::new(
        spec.parametrization_implementation,
        spec.parametrization_params.clone(),
    )
    .build()
    .map_err(|err| StoreError::store(format!("failed to build parametrization: {err}")))?;
    parametrization
        .validate_point_spec(&spec.point_spec)
        .map_err(|err| {
            StoreError::store(format!(
                "incompatible parametrization for point_spec on run {}: {}",
                worker.run_id, err
            ))
        })?;

    worker
        .register_active_worker(spec.evaluator_implementation.as_ref())
        .await?;
    worker
        .store
        .assign_evaluator(worker.run_id, &worker.worker_id)
        .await?;

    info!(
        target: "worker_log",
        run_id = worker.run_id,
        node_id = %worker.node_id,
        worker_id = %worker.worker_id,
        role = %WorkerRole::Evaluator,
        event_type = "worker_started",
        "evaluator worker started"
    );

    let mut runner = EvaluatorRunner::new(
        worker.run_id,
        worker.worker_id.clone(),
        evaluator,
        parametrization,
        observable_factory,
        spec.point_spec.clone(),
        Duration::from_millis(
            spec.evaluator_runner_params
                .performance_snapshot_interval_ms,
        ),
        worker.store.clone(),
    );

    let idle_backoff = Duration::from_millis(spec.evaluator_runner_params.min_loop_time_ms);

    loop {
        if *stop_rx.borrow() {
            break;
        }

        worker.heartbeat_with_log().await;

        let sleep_after = match runner.tick().await {
            Ok(tick) => {
                if tick.processed_samples > 0 {
                    Duration::ZERO
                } else {
                    idle_backoff
                }
            }
            Err(err) => {
                warn!(
                    target: "worker_log",
                    run_id = worker.run_id,
                    node_id = %worker.node_id,
                    worker_id = %worker.worker_id,
                    role = %WorkerRole::Evaluator,
                    event_type = "tick_failed",
                    error = %err,
                    "evaluator tick failed"
                );
                idle_backoff
            }
        };

        if sleep_after > Duration::ZERO {
            tokio::select! {
                _ = stop_rx.changed() => {}
                _ = sleep(sleep_after) => {}
            }
        }
    }

    if let Err(err) = worker
        .store
        .unassign_evaluator(worker.run_id, &worker.worker_id)
        .await
    {
        warn!(
            target: "worker_log",
            run_id = worker.run_id,
            node_id = %worker.node_id,
            worker_id = %worker.worker_id,
            role = %WorkerRole::Evaluator,
            event_type = "unassign_failed",
            error = %err,
            "failed to unassign evaluator"
        );
    }

    worker.mark_inactive_with_log().await;
    info!(
        target: "worker_log",
        run_id = worker.run_id,
        node_id = %worker.node_id,
        worker_id = %worker.worker_id,
        role = %WorkerRole::Evaluator,
        event_type = "worker_stopped",
        "evaluator worker stopped"
    );

    Ok(())
}
