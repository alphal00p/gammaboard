use crate::core::{StoreError, Worker as WorkerRecord, WorkerRole, WorkerStatus};
use crate::engines::observable::ObservableFactory;
use crate::engines::{EvaluatorFactory, ParametrizationFactory, SamplerAggregatorFactory};
use crate::runners::{
    evaluator::EvaluatorRunner,
    sampler_aggregator::{RunnerConfig, SamplerAggregatorRunner},
};
use serde_json::json;
use std::time::Duration;
use tokio::{sync::watch, time::sleep};
use tracing::{info, warn};

use super::{NodeRunnerStore, binary_version};

pub(super) struct ActiveWorker<S: NodeRunnerStore> {
    store: S,
    node_id: String,
    worker_id: String,
    role: WorkerRole,
    run_id: i32,
}

impl<S: NodeRunnerStore> ActiveWorker<S> {
    pub(super) fn new(
        store: S,
        node_id: impl Into<String>,
        worker_id: impl Into<String>,
        role: WorkerRole,
        run_id: i32,
    ) -> Self {
        Self {
            store,
            node_id: node_id.into(),
            worker_id: worker_id.into(),
            role,
            run_id,
        }
    }

    pub(super) async fn run(self, stop_rx: watch::Receiver<bool>) -> Result<(), StoreError> {
        match self.role {
            WorkerRole::Evaluator => self.run_evaluator(stop_rx).await,
            WorkerRole::SamplerAggregator => self.run_sampler_aggregator(stop_rx).await,
        }
    }

    async fn run_evaluator(self, mut stop_rx: watch::Receiver<bool>) -> Result<(), StoreError> {
        let Some(spec) = self.store.load_run_spec(self.run_id).await? else {
            warn!(
                target: "worker_log",
                run_id = self.run_id,
                node_id = %self.node_id,
                worker_id = %self.worker_id,
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
                    self.run_id, err
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
                self.run_id, spec.evaluator_implementation, spec.observable_implementation
            )));
        }

        let evaluator_init_metadata = evaluator.get_init_metadata();
        if self
            .store
            .try_set_evaluator_init_metadata(self.run_id, &evaluator_init_metadata)
            .await?
        {
            info!(
                target: "worker_log",
                run_id = self.run_id,
                node_id = %self.node_id,
                worker_id = %self.worker_id,
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
                    self.run_id, err
                ))
            })?;

        self.register_active_worker(spec.evaluator_implementation.as_ref())
            .await?;
        self.store
            .assign_evaluator(self.run_id, &self.worker_id)
            .await?;

        info!(
            target: "worker_log",
            run_id = self.run_id,
            node_id = %self.node_id,
            worker_id = %self.worker_id,
            role = %WorkerRole::Evaluator,
            event_type = "worker_started",
            "evaluator worker started"
        );

        let mut runner = EvaluatorRunner::new(
            self.run_id,
            self.worker_id.clone(),
            evaluator,
            parametrization,
            observable_factory,
            spec.point_spec.clone(),
            Duration::from_millis(
                spec.evaluator_runner_params
                    .performance_snapshot_interval_ms,
            ),
            self.store.clone(),
        );

        let idle_backoff = Duration::from_millis(spec.evaluator_runner_params.min_loop_time_ms);

        loop {
            if *stop_rx.borrow() {
                break;
            }

            self.heartbeat_with_log().await;

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
                        run_id = self.run_id,
                        node_id = %self.node_id,
                        worker_id = %self.worker_id,
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

        if let Err(err) = self
            .store
            .unassign_evaluator(self.run_id, &self.worker_id)
            .await
        {
            warn!(
                target: "worker_log",
                run_id = self.run_id,
                node_id = %self.node_id,
                worker_id = %self.worker_id,
                role = %WorkerRole::Evaluator,
                event_type = "unassign_failed",
                error = %err,
                "failed to unassign evaluator"
            );
        }

        self.mark_inactive_with_log().await;
        info!(
            target: "worker_log",
            run_id = self.run_id,
            node_id = %self.node_id,
            worker_id = %self.worker_id,
            role = %WorkerRole::Evaluator,
            event_type = "worker_stopped",
            "evaluator worker stopped"
        );

        Ok(())
    }

    async fn run_sampler_aggregator(
        self,
        mut stop_rx: watch::Receiver<bool>,
    ) -> Result<(), StoreError> {
        let Some(spec) = self.store.load_run_spec(self.run_id).await? else {
            warn!(
                target: "worker_log",
                run_id = self.run_id,
                node_id = %self.node_id,
                worker_id = %self.worker_id,
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
                    self.run_id, err
                ))
            })?;
        let mut engine = engine;
        let sampler_init_metadata = engine.get_init_metadata();
        if self
            .store
            .try_set_sampler_init_metadata(self.run_id, &sampler_init_metadata)
            .await?
        {
            info!(
                target: "worker_log",
                run_id = self.run_id,
                node_id = %self.node_id,
                worker_id = %self.worker_id,
                role = %WorkerRole::SamplerAggregator,
                event_type = "init_metadata_written",
                "stored sampler-aggregator init metadata"
            );
        }

        let observable_factory =
            ObservableFactory::new(spec.observable_implementation, spec.observable_params);

        self.register_active_worker(spec.sampler_aggregator_implementation.as_ref())
            .await?;

        info!(
            target: "worker_log",
            run_id = self.run_id,
            node_id = %self.node_id,
            worker_id = %self.worker_id,
            role = %WorkerRole::SamplerAggregator,
            event_type = "worker_started",
            "sampler-aggregator worker started"
        );

        let mut runner = SamplerAggregatorRunner::new(
            self.run_id,
            self.worker_id.clone(),
            engine,
            observable_factory,
            self.store.clone(),
            self.store.clone(),
            RunnerConfig {
                nr_samples: spec.sampler_aggregator_runner_params.nr_samples,
                performance_snapshot_interval_ms: spec
                    .sampler_aggregator_runner_params
                    .performance_snapshot_interval_ms,
                max_batches_per_tick: spec.sampler_aggregator_runner_params.max_batches_per_tick,
                max_pending_batches: spec.sampler_aggregator_runner_params.max_pending_batches,
                completed_batch_fetch_limit: spec
                    .sampler_aggregator_runner_params
                    .completed_batch_fetch_limit,
            },
            spec.point_spec.clone(),
        )
        .await
        .map_err(|err| StoreError::store(err.to_string()))?;

        let lease_ttl = Duration::from_millis(spec.sampler_aggregator_runner_params.lease_ttl_ms);
        let interval = Duration::from_millis(spec.sampler_aggregator_runner_params.interval_ms);
        let mut owns_lease = false;

        loop {
            if *stop_rx.borrow() {
                break;
            }

            self.heartbeat_with_log().await;

            let lease_result = if owns_lease {
                self.store
                    .renew_sampler_aggregator_lease(self.run_id, &self.worker_id, lease_ttl)
                    .await
            } else {
                self.store
                    .acquire_sampler_aggregator_lease(self.run_id, &self.worker_id, lease_ttl)
                    .await
            };

            match lease_result {
                Ok(has_lease) => owns_lease = has_lease,
                Err(err) => {
                    warn!(
                        target: "worker_log",
                        run_id = self.run_id,
                        node_id = %self.node_id,
                        worker_id = %self.worker_id,
                        role = %WorkerRole::SamplerAggregator,
                        event_type = "lease_operation_failed",
                        error = %err,
                        "lease operation failed"
                    );
                    owns_lease = false;
                }
            }

            if owns_lease && let Err(err) = runner.tick().await {
                warn!(
                    target: "worker_log",
                    run_id = self.run_id,
                    node_id = %self.node_id,
                    worker_id = %self.worker_id,
                    role = %WorkerRole::SamplerAggregator,
                    event_type = "tick_failed",
                    error = %err,
                    "sampler-aggregator tick failed"
                );
            }

            tokio::select! {
                _ = stop_rx.changed() => {}
                _ = sleep(interval) => {}
            }
        }

        if owns_lease
            && let Err(err) = self
                .store
                .release_sampler_aggregator_lease(self.run_id, &self.worker_id)
                .await
        {
            warn!(
                target: "worker_log",
                run_id = self.run_id,
                node_id = %self.node_id,
                worker_id = %self.worker_id,
                role = %WorkerRole::SamplerAggregator,
                event_type = "lease_release_failed",
                error = %err,
                "failed to release sampler-aggregator lease"
            );
        }

        self.mark_inactive_with_log().await;
        info!(
            target: "worker_log",
            run_id = self.run_id,
            node_id = %self.node_id,
            worker_id = %self.worker_id,
            role = %WorkerRole::SamplerAggregator,
            event_type = "worker_stopped",
            "sampler-aggregator worker stopped"
        );

        Ok(())
    }

    async fn register_active_worker(&self, implementation: &str) -> Result<(), StoreError> {
        self.store
            .register_worker(&WorkerRecord {
                worker_id: self.worker_id.clone(),
                node_id: Some(self.node_id.clone()),
                role: self.role,
                implementation: implementation.to_string(),
                version: binary_version().to_string(),
                node_specs: json!({ "node_id": self.node_id }),
                status: WorkerStatus::Active,
                last_seen: None,
            })
            .await
    }

    async fn heartbeat_with_log(&self) {
        if let Err(err) = self.store.heartbeat_worker(&self.worker_id).await {
            warn!(
                target: "worker_log",
                run_id = self.run_id,
                worker_id = %self.worker_id,
                role = %self.role,
                event_type = "heartbeat_failed",
                error = %err,
                "worker heartbeat failed"
            );
        }
    }

    async fn mark_inactive_with_log(&self) {
        if let Err(err) = self
            .store
            .update_worker_status(&self.worker_id, WorkerStatus::Inactive)
            .await
        {
            warn!(
                target: "worker_log",
                run_id = self.run_id,
                worker_id = %self.worker_id,
                role = %self.role,
                event_type = "worker_inactive_failed",
                error = %err,
                "failed to mark worker inactive"
            );
        }
    }
}
