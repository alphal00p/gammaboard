//! Bounded direct baseline. Uses the production evaluator, uniform sampler,
//! identity materializer and scalar accumulator, without a database or work queue.
use crate::core::{AccumulatorConfig, EvaluatorConfig, SamplerAggregatorConfig};
use crate::evaluation::{AccumulatorState, EvalBatchOptions, Evaluator, Materializer};
use crate::sampling::{IdentityMaterializer, NaiveMonteCarloSamplerParams, SamplerAggregator};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workload {
    pub evaluator: EvaluatorConfig,
}

struct Worker {
    evaluator: Box<dyn Evaluator>,
    sampler: Box<dyn SamplerAggregator>,
    materializer: IdentityMaterializer,
    accumulator: AccumulatorState,
}
impl Worker {
    fn new(config: &EvaluatorConfig, seed: u64) -> Result<Self> {
        let evaluator = config.build()?;
        config.validate_accumulator_config(&AccumulatorConfig::scalar())?;
        let sampler = SamplerAggregatorConfig::NaiveMonteCarlo {
            params: NaiveMonteCarloSamplerParams {
                seed,
                ..Default::default()
            },
            materializer: None,
        }
        .build(evaluator.get_domain(), None, None, evaluator.metadata())?;
        Ok(Self {
            evaluator,
            sampler,
            materializer: IdentityMaterializer::new(),
            accumulator: AccumulatorState::from_config(&AccumulatorConfig::scalar()),
        })
    }
    fn batch(&mut self, size: usize) -> Result<()> {
        let latent = self
            .sampler
            .produce_latent_batch(size)?
            .with_accumulator_config(AccumulatorConfig::scalar())
            .build();
        let batch = self.materializer.materialize_batch(&latent)?;
        let result = self.evaluator.eval_batch(
            &batch,
            &AccumulatorConfig::scalar(),
            EvalBatchOptions {
                require_training_values: false,
            },
        )?;
        self.accumulator.merge(result.accumulator)?;
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct DirectMeasurement {
    pub schema_version: u32,
    pub backend: &'static str,
    pub workers: usize,
    pub batch_size: usize,
    pub completed_samples: usize,
    pub elapsed_seconds: f64,
    pub samples_per_second: f64,
    pub initialization_seconds: f64,
    pub total_seconds: f64,
    pub workload: Workload,
    pub provenance: crate::provenance::RunProvenance,
}

pub fn direct(
    workload: Workload,
    workers: usize,
    batch_size: usize,
    duration: Duration,
    warmup: Duration,
    samples: Option<usize>,
) -> Result<DirectMeasurement> {
    ensure!(
        workers > 0 && workers <= 1024 && batch_size > 0 && batch_size <= 1_000_000,
        "invalid workers or batch size"
    );
    ensure!(samples.is_none_or(|n| n > 0), "samples must be positive");
    let total_start = Instant::now();
    let runtimes = (0..workers)
        .map(|i| Worker::new(&workload.evaluator, 1234 + i as u64))
        .collect::<Result<Vec<_>>>()?;
    let initialization_seconds = total_start.elapsed().as_secs_f64();
    let barrier = Arc::new(Barrier::new(workers + 1));
    let remaining = AtomicUsize::new(samples.unwrap_or(0));
    let (completed_samples, elapsed_seconds) = std::thread::scope(|scope| -> Result<_> {
        let mut handles = Vec::new();
        for mut worker in runtimes {
            let barrier = barrier.clone();
            let remaining = &remaining;
            handles.push(scope.spawn(move || -> Result<(usize, Instant, Instant)> {
                let warmup_start = Instant::now();
                let warmed =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                        while warmup_start.elapsed() < warmup {
                            worker.batch(batch_size)?;
                        }
                        Ok(())
                    }))
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("benchmark warmup panicked")));
                // Rendezvous even after a warmup failure, so other threads cannot deadlock.
                barrier.wait();
                warmed?;
                let start = Instant::now();
                let mut completed = 0;
                loop {
                    let size = if samples.is_some() {
                        let previous = remaining
                            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                                n.checked_sub(n.min(batch_size))
                            })
                            .unwrap();
                        previous.min(batch_size)
                    } else if start.elapsed() < duration {
                        batch_size
                    } else {
                        0
                    };
                    if size == 0 {
                        break;
                    }
                    worker.batch(size)?;
                    completed += size;
                }
                std::hint::black_box(worker.accumulator);
                Ok((completed, start, Instant::now()))
            }));
        }
        barrier.wait();
        let mut first = None::<Instant>;
        let mut last = None::<Instant>;
        let mut total = 0;
        for handle in handles {
            let (count, start, end) = handle
                .join()
                .map_err(|_| anyhow::anyhow!("benchmark worker panicked"))??;
            total += count;
            first = Some(first.map_or(start, |v| v.min(start)));
            last = Some(last.map_or(end, |v| v.max(end)));
        }
        Ok((
            total,
            last.unwrap().duration_since(first.unwrap()).as_secs_f64(),
        ))
    })?;
    Ok(DirectMeasurement {
        schema_version: 1,
        backend: "direct_uniform_scalar",
        workers,
        batch_size,
        completed_samples,
        elapsed_seconds,
        samples_per_second: completed_samples as f64 / elapsed_seconds,
        initialization_seconds,
        total_seconds: total_start.elapsed().as_secs_f64(),
        provenance: crate::provenance::RunProvenance::capture(None, toml::to_string(&workload)?),
        workload,
    })
}

/// Calibrate once; callers must reuse the returned iteration count at every core count.
pub fn calibrate(target: Duration) -> Result<serde_json::Value> {
    ensure!(
        target >= Duration::from_micros(1) && target <= Duration::from_millis(100),
        "calibration target must be 1us..100ms"
    );
    let mut iterations = 1000_u64;
    let mut per_iteration = 0.0;
    for _ in 0..8 {
        let start = Instant::now();
        crate::evaluation::evaluator::unit::cpu_work(iterations, 64);
        let elapsed = start.elapsed().as_secs_f64();
        if elapsed >= 0.02 {
            per_iteration = elapsed / (64.0 * iterations as f64);
            break;
        }
        iterations = iterations.saturating_mul(4);
    }
    ensure!(per_iteration > 0.0, "CPU calibration failed");
    let fixed = (target.as_secs_f64() / per_iteration).round().max(1.0) as u64;
    let start = Instant::now();
    crate::evaluation::evaluator::unit::cpu_work(fixed, 64);
    Ok(
        json!({"schema_version":1,"requested_seconds_per_sample":target.as_secs_f64(),
        "cpu_iterations_per_sample":fixed,"measured_seconds_per_sample":start.elapsed().as_secs_f64()/64.0}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn direct_counts_partial_batches_across_workers() {
        let workload = Workload {
            evaluator: EvaluatorConfig::Unit {
                params: Default::default(),
            },
        };
        let result = direct(workload, 3, 16, Duration::ZERO, Duration::ZERO, Some(101)).unwrap();
        assert_eq!(result.completed_samples, 101);
        assert!(result.samples_per_second.is_finite());
    }
}
