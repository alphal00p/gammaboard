//! Mock sampler-aggregator binary.
//!
//! This binary implements `SamplerAggregatorEngine` and simulates a training
//! phase by adding latency for early samples. After the configured training
//! budget is consumed, aggregation becomes effectively instant.

use gammaboard::{
    AssignmentLeaseStore, Batch, CompletedBatch, ComponentInstance, ComponentRegistryStore,
    ComponentRole, EngineError, EngineState, InstanceStatus, PgStore, RunnerConfig,
    SamplerAggregatorEngine, SamplerAggregatorRunner, WeightedPoint, get_pg_pool,
};
use rand::Rng;
use serde_json::json;
use std::{thread, time::Duration};

const RUN_ID: i32 = 1;
const INSTANCE_ID: &str = "mock-sampler-aggregator-1";
const INTERVAL_MS: u64 = 500;
const LEASE_TTL_MS: u64 = 5_000;
const MAX_BATCHES_PER_TICK: usize = 1;
const MAX_PENDING_BATCHES: usize = 128;
const COMPLETED_BATCH_FETCH_LIMIT: usize = 512;
const BATCH_SIZE: usize = 64;
const TRAINING_TARGET_SAMPLES: usize = 2_000;
const TRAINING_DELAY_PER_SAMPLE_MS: u64 = 2;

struct MockTrainingEngine {
    training_target_samples: usize,
    training_delay_per_sample_ms: u64,
    batch_size: usize,
    trained_samples: usize,
    total_batches: usize,
    total_samples: usize,
    total_sum: f64,
}

impl MockTrainingEngine {
    fn new(
        training_target_samples: usize,
        training_delay_per_sample_ms: u64,
        batch_size: usize,
    ) -> Self {
        Self {
            training_target_samples,
            training_delay_per_sample_ms,
            batch_size,
            trained_samples: 0,
            total_batches: 0,
            total_samples: 0,
            total_sum: 0.0,
        }
    }
}

impl SamplerAggregatorEngine for MockTrainingEngine {
    fn implementation(&self) -> &'static str {
        "mock_sampler_aggregator"
    }

    fn version(&self) -> &'static str {
        "v1"
    }

    fn init(&mut self, _state: Option<EngineState>) -> Result<(), EngineError> {
        Ok(())
    }

    fn produce_batches(&mut self, max_batches: usize) -> Result<Vec<Batch>, EngineError> {
        let mut rng = rand::thread_rng();
        let mut out = Vec::with_capacity(max_batches);

        for _ in 0..max_batches {
            let mut points = Vec::with_capacity(self.batch_size);
            for _ in 0..self.batch_size {
                let x = rng.r#gen::<f64>() * 10.0;
                let w = 0.5 + rng.r#gen::<f64>();
                points.push(WeightedPoint::new(json!(x), w));
            }
            out.push(Batch::new(points));
        }

        Ok(out)
    }

    fn ingest_completed(&mut self, completed: &[CompletedBatch]) -> Result<(), EngineError> {
        let mut new_samples = 0usize;
        let mut batch_sum = 0.0f64;
        let mut batch_count = 0usize;

        for batch in completed {
            if batch.results.values.len() != batch.batch.points.len() {
                continue;
            }
            batch_count += 1;
            new_samples += batch.results.values.len();
            batch_sum += batch.results.values.iter().sum::<f64>();
        }

        if batch_count == 0 {
            return Ok(());
        }

        let remaining_training = self
            .training_target_samples
            .saturating_sub(self.trained_samples);
        let delayed_samples = remaining_training.min(new_samples);

        if delayed_samples > 0 && self.training_delay_per_sample_ms > 0 {
            let total_delay_ms = delayed_samples as u64 * self.training_delay_per_sample_ms;
            thread::sleep(Duration::from_millis(total_delay_ms));
        }

        self.trained_samples = self.trained_samples.saturating_add(new_samples);
        self.total_batches += batch_count;
        self.total_samples += new_samples;
        self.total_sum += batch_sum;

        let mode = if self.trained_samples < self.training_target_samples {
            "training"
        } else {
            "serving"
        };
        let mean = if self.total_samples > 0 {
            self.total_sum / self.total_samples as f64
        } else {
            0.0
        };

        println!(
            "🧪 mode={mode} processed_batches={batch_count} processed_samples={new_samples} total_samples={} mean={mean:.6}",
            self.total_samples
        );

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "🧪 Starting mock sampler-aggregator: run_id={} instance_id={}",
        RUN_ID, INSTANCE_ID
    );

    let pool = get_pg_pool(5).await?;
    let store = PgStore::new(pool);

    let engine = MockTrainingEngine::new(
        TRAINING_TARGET_SAMPLES,
        TRAINING_DELAY_PER_SAMPLE_MS,
        BATCH_SIZE,
    );
    let implementation = engine.implementation().to_string();
    let version = engine.version().to_string();

    store
        .register_instance(&ComponentInstance {
            instance_id: INSTANCE_ID.to_string(),
            node_id: None,
            role: ComponentRole::SamplerAggregator,
            implementation,
            version,
            node_specs: json!({ "mock": true }),
            status: InstanceStatus::Active,
            last_seen: None,
        })
        .await?;

    let config = RunnerConfig {
        max_batches_per_tick: MAX_BATCHES_PER_TICK,
        max_pending_batches: MAX_PENDING_BATCHES,
        completed_batch_fetch_limit: COMPLETED_BATCH_FETCH_LIMIT,
    };
    let mut runner = SamplerAggregatorRunner::new(
        RUN_ID,
        engine,
        store.clone(),
        store.clone(),
        store.clone(),
        config,
    )
    .await?;

    let lease_ttl = Duration::from_millis(LEASE_TTL_MS);
    let mut owns_lease = false;

    loop {
        store.heartbeat_instance(INSTANCE_ID).await?;

        owns_lease = if owns_lease {
            store
                .renew_sampler_aggregator_lease(RUN_ID, INSTANCE_ID, lease_ttl)
                .await?
        } else {
            store
                .acquire_sampler_aggregator_lease(RUN_ID, INSTANCE_ID, lease_ttl)
                .await?
        };

        if owns_lease {
            let tick = runner.tick().await?;
            if tick.enqueued_batches > 0 || tick.processed_completed_batches > 0 {
                println!(
                    "🔁 run={} enqueued={} processed_completed={} cursor={:?}",
                    RUN_ID,
                    tick.enqueued_batches,
                    tick.processed_completed_batches,
                    tick.last_processed_batch_id
                );
            }
        } else {
            println!("⏳ waiting for sampler-aggregator lease on run {}", RUN_ID);
        }

        tokio::time::sleep(Duration::from_millis(INTERVAL_MS)).await;
    }
}
