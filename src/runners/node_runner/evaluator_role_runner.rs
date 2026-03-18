use crate::core::StoreError;
use crate::runners::evaluator::EvaluatorRunner;
use crate::sampling::ParametrizationBuildContext;
use std::time::Duration;
use tokio::{sync::watch, time::sleep};
use tracing::Instrument;
use tracing::{info, warn};

use super::{NodeRunnerStore, active_worker::ActiveWorker};

pub(crate) async fn run_evaluator_role<S: NodeRunnerStore>(
    worker: &ActiveWorker<S>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<(), StoreError> {
    let Some(spec) = worker.store.load_run_spec(worker.run_id).await? else {
        warn!("run has no RunSpec; evaluator not started");
        return Ok(());
    };

    let engine_span = tracing::span!(tracing::Level::TRACE, "evaluator_engine_context");
    let evaluator = {
        let _engine_scope = engine_span.enter();
        let evaluator = spec
            .evaluator
            .build()
            .map_err(|err| StoreError::store(format!("failed to build evaluator: {err}")))?;

        let parametrization = spec
            .parametrization
            .build(ParametrizationBuildContext::default())
            .map_err(|err| StoreError::store(format!("failed to build parametrization: {err}")))?;
        parametrization
            .validate_point_spec(&spec.point_spec)
            .map_err(|err| {
                StoreError::store(format!(
                    "incompatible parametrization for point_spec on run {}: {}",
                    worker.run_id, err
                ))
            })?;

        evaluator
    };

    let _ = spec.evaluator.kind_str();
    worker.mark_active_with_log().await?;

    info!("evaluator worker started");

    let mut runner = EvaluatorRunner::new(
        worker.run_id,
        worker.node_id.clone(),
        evaluator,
        spec.point_spec.clone(),
        Duration::from_millis(
            spec.evaluator_runner_params
                .performance_snapshot_interval_ms,
        ),
        worker.store.clone(),
        worker.store.clone(),
    );

    let idle_backoff = Duration::from_millis(spec.evaluator_runner_params.min_loop_time_ms);

    loop {
        if *stop_rx.borrow() {
            break;
        }

        worker.heartbeat_with_log().await;

        let sleep_after = match runner.tick().instrument(engine_span.clone()).await {
            Ok(tick) => {
                if tick.processed_samples > 0 {
                    Duration::ZERO
                } else {
                    idle_backoff
                }
            }
            Err(err) => {
                warn!("evaluator tick failed: {err}");
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

    worker.mark_inactive_with_log().await;
    info!("evaluator worker stopped");

    Ok(())
}
