//! Sampler-aggregator runner binary.
//!
//! This first version wires up the generic runner and store contracts.
//! The local engine below is intentionally minimal and can be replaced by a
//! real adaptive sampler-aggregator engine implementation.

use gammaboard::{
    AssignmentLeaseStore, ComponentInstance, ComponentRegistryStore, ComponentRole, EngineError,
    EngineState, InstanceStatus, PgStore, RunnerConfig, SamplerAggregatorEngine,
    SamplerAggregatorRunner, get_pg_pool,
};
use serde_json::json;
use std::{env, time::Duration};

struct InMemoryStatsEngine {
    nr_batches: i64,
    nr_samples: i64,
    sum: f64,
}

impl InMemoryStatsEngine {
    fn new() -> Self {
        Self {
            nr_batches: 0,
            nr_samples: 0,
            sum: 0.0,
        }
    }
}

impl SamplerAggregatorEngine for InMemoryStatsEngine {
    fn implementation(&self) -> &'static str {
        "in_memory_stats"
    }

    fn version(&self) -> &'static str {
        "v1"
    }

    fn init(&mut self, _state: Option<EngineState>) -> Result<(), EngineError> {
        Ok(())
    }

    fn produce_batches(
        &mut self,
        _max_batches: usize,
    ) -> Result<Vec<gammaboard::Batch>, EngineError> {
        Ok(Vec::new())
    }

    fn ingest_completed(
        &mut self,
        completed: &[gammaboard::CompletedBatch],
    ) -> Result<(), EngineError> {
        for batch in completed {
            if batch.results.values.len() != batch.batch.points.len() {
                continue;
            }
            self.nr_batches += 1;
            self.nr_samples += batch.results.values.len() as i64;
            self.sum += batch.results.values.iter().sum::<f64>();
        }

        let mean = if self.nr_samples > 0 {
            self.sum / self.nr_samples as f64
        } else {
            0.0
        };
        println!(
            "📈 in-memory engine: batches={}, samples={}, mean={mean:.6}",
            self.nr_batches, self.nr_samples
        );

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🧮 Starting sampler-aggregator runner...");

    let run_id = env::var("RUN_ID")
        .ok()
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(1);
    let instance_id =
        env::var("SAMPLER_AGGREGATOR_ID").unwrap_or_else(|_| "sampler-aggregator-1".to_string());
    let interval_ms = env::var("AGGREGATOR_INTERVAL_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1000);
    let lease_ttl_ms = env::var("SAMPLER_LEASE_TTL_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(5_000);
    let max_pending_batches = env::var("AGGREGATOR_MAX_PENDING_BATCHES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(128);

    let pool = get_pg_pool(5).await?;
    let store = PgStore::new(pool);

    let engine = InMemoryStatsEngine::new();
    let implementation = engine.implementation().to_string();
    let version = engine.version().to_string();

    store
        .register_instance(&ComponentInstance {
            instance_id: instance_id.clone(),
            node_id: None,
            role: ComponentRole::SamplerAggregator,
            implementation,
            version,
            node_specs: json!({}),
            status: InstanceStatus::Active,
            last_seen: None,
        })
        .await?;

    let mut runner = SamplerAggregatorRunner::new(
        run_id,
        engine,
        store.clone(),
        store.clone(),
        store.clone(),
        RunnerConfig {
            max_pending_batches,
            ..RunnerConfig::default()
        },
    )
    .await?;
    let lease_ttl = Duration::from_millis(lease_ttl_ms);
    let mut owns_lease = false;

    loop {
        store.heartbeat_instance(&instance_id).await?;

        owns_lease = if owns_lease {
            store
                .renew_sampler_aggregator_lease(run_id, &instance_id, lease_ttl)
                .await?
        } else {
            store
                .acquire_sampler_aggregator_lease(run_id, &instance_id, lease_ttl)
                .await?
        };

        if owns_lease {
            let tick = runner.tick().await?;
            if tick.enqueued_batches > 0 || tick.processed_completed_batches > 0 {
                println!(
                    "🔁 run={} enqueued={} processed_completed={} cursor={:?}",
                    run_id,
                    tick.enqueued_batches,
                    tick.processed_completed_batches,
                    tick.last_processed_batch_id
                );
            }
        } else {
            println!("⏳ waiting for sampler-aggregator lease on run {}", run_id);
        }

        tokio::time::sleep(Duration::from_millis(interval_ms)).await;
    }
}
