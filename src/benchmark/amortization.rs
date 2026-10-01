//! Direct in-memory reference for the same engines used by the runner benchmark.
//! Bounded work, ordered generation feedback, and steady-state completion timing.
use crate::{
    Domain, EvalBatchOptions,
    core::{AccumulatorConfig, EvaluatorConfig, SamplerAggregatorConfig},
    evaluation::{AccumulatorState, BatchResult},
    sampling::LatentBatch,
};
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::mpsc, time::Instant};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub batch_size: usize,
    pub generation_size: usize,
    pub cpu_iterations_per_sample: u64,
    pub feedback: bool,
    pub duration_seconds: f64,
    pub evaluator_cpus: Vec<usize>,
    pub sampler_cpus: Vec<usize>,
}

pub fn measure(config: Config) -> Result<Value> {
    ensure!(
        config.batch_size >= 16
            && config.generation_size >= config.batch_size
            && config.generation_size.is_multiple_of(config.batch_size),
        "generation size must be a positive multiple of batch size"
    );
    ensure!(
        config.duration_seconds.is_finite() && (0.01..=120.).contains(&config.duration_seconds),
        "invalid measurement duration"
    );
    ensure!(
        !config.evaluator_cpus.is_empty()
            && !config.sampler_cpus.is_empty()
            && config
                .evaluator_cpus
                .iter()
                .all(|cpu| !config.sampler_cpus.contains(cpu)),
        "separate sampler/evaluator CPU allocations are required"
    );
    super::pin(&config.sampler_cpus)?;
    let evaluator: EvaluatorConfig = serde_json::from_value(json!({
        "kind":"unit", "continuous_dims":6, "value_coordinate":0,
        "cpu_iterations_per_sample":config.cpu_iterations_per_sample
    }))?;
    let definition: SamplerAggregatorConfig = serde_json::from_value(json!({
        "kind":"naive_monte_carlo", "seed":1234,
        "generation_batch_size":config.generation_size,
        "training_window_samples":if config.feedback { 1_000_000_000_000_usize } else { 0 }
    }))?;
    let mut sampler = definition.build(Domain::continuous(6), None, None, json!({}))?;
    let accumulator_config = AccumulatorConfig::scalar();
    let mut accumulator = AccumulatorState::from_config(&accumulator_config);
    let workers = config.evaluator_cpus.len();
    std::thread::scope(|scope| -> Result<Value> {
        let (completed_tx, completed_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let mut inputs = Vec::new();
        for &cpu in &config.evaluator_cpus {
            let (tx, rx) = mpsc::sync_channel::<(usize, LatentBatch)>(2);
            inputs.push(tx);
            let completed_tx = completed_tx.clone();
            let ready_tx = ready_tx.clone();
            let evaluator = evaluator.clone();
            let accumulator_config = accumulator_config.clone();
            let feedback = config.feedback;
            scope.spawn(move || {
                let initialized = super::pin(&[cpu]).and_then(|_| Ok(evaluator.build()?));
                let mut evaluator = match initialized {
                    Ok(evaluator) => {
                        ready_tx.send(Ok(())).ok();
                        evaluator
                    }
                    Err(error) => {
                        ready_tx.send(Err(error)).ok();
                        return;
                    }
                };
                while let Ok((id, latent)) = rx.recv() {
                    let result = (|| -> Result<(BatchResult, f64)> {
                        let batch = latent.payload.into_batch()?;
                        let start = Instant::now();
                        let result = evaluator.eval_batch(
                            &batch,
                            &accumulator_config,
                            EvalBatchOptions {
                                require_training_values: feedback,
                            },
                        )?;
                        let seconds = start.elapsed().as_secs_f64();
                        Ok((result, seconds))
                    })();
                    if completed_tx.send((id, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(ready_tx);
        drop(completed_tx);
        for _ in 0..workers {
            ready_rx.recv()??;
        }
        let batches_per_generation = config.generation_size / config.batch_size;
        let mut generation = None;
        let mut cursor = config.generation_size;
        let mut sent = 0;
        let mut retired = 0;
        let mut pending = BTreeMap::new();
        let mut feedback = Vec::new();
        let mut accepted = 0;
        let mut start = None;
        let mut completed_at_start = 0;
        let mut evaluate_seconds = 0.;
        let mut evaluate_batches = 0;
        loop {
            while sent - retired < 2 * workers {
                if cursor == config.generation_size {
                    generation = Some(sampler.generate(None)?.into_batch()?.build());
                    cursor = 0;
                }
                let batch =
                    generation
                        .as_ref()
                        .unwrap()
                        .slice_at(cursor, config.batch_size, cursor * 6)?;
                inputs[sent % workers].send((sent, batch))?;
                cursor += config.batch_size;
                sent += 1;
            }
            let (id, result) = completed_rx.recv()?;
            pending.insert(id, result?);
            while let Some((result, seconds)) = pending.remove(&retired) {
                accumulator.merge(result.accumulator)?;
                if let Some(values) = result.values {
                    feedback.extend(values);
                }
                retired += 1;
                evaluate_seconds += seconds;
                evaluate_batches += 1;
                if !retired.is_multiple_of(batches_per_generation) {
                    continue;
                }
                if config.feedback {
                    ensure!(
                        feedback.len() == config.generation_size,
                        "incomplete generation feedback"
                    );
                    sampler.feedback(&feedback)?;
                    feedback.clear();
                }
                accepted += config.generation_size;
                let now = Instant::now();
                if let Some(begin) = start {
                    let elapsed = now.duration_since(begin).as_secs_f64();
                    if elapsed >= config.duration_seconds {
                        let measured = accepted - completed_at_start;
                        ensure!(
                            accumulator.sample_count() as usize == accepted,
                            "direct accumulator count differs from accepted work"
                        );
                        return Ok(json!({
                            "samples":measured, "elapsed_seconds":elapsed,
                            "rate":measured as f64 / elapsed,
                            "evaluate_seconds":evaluate_seconds,
                            "evaluate_batches":evaluate_batches,
                            "mean_evaluate_batch_seconds":evaluate_seconds / evaluate_batches as f64,
                            "sampler_diagnostics":sampler.get_diagnostics(),
                            "scope":"Native engines in a bounded in-memory pipeline; generation, partitioning, materialization, evaluation, accumulation and ordered generation feedback. One warmup generation; full-generation completion endpoints. Startup and final drain excluded."
                        }));
                    }
                } else {
                    start = Some(now);
                    completed_at_start = accepted;
                    evaluate_seconds = 0.;
                    evaluate_batches = 0;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_pipeline_preserves_complete_generation_feedback() {
        let allowed = (0..libc::CPU_SETSIZE as usize)
            .filter(|&cpu| {
                let mut set = unsafe { std::mem::zeroed::<libc::cpu_set_t>() };
                unsafe {
                    libc::sched_getaffinity(0, std::mem::size_of_val(&set), &mut set) == 0
                        && libc::CPU_ISSET(cpu, &set)
                }
            })
            .take(3)
            .collect::<Vec<_>>();
        if allowed.len() < 3 {
            return;
        }
        for feedback in [false, true] {
            // Affinity changes belong to a short-lived thread, not the test harness.
            let cpus = allowed.clone();
            let result = std::thread::spawn(move || {
                measure(Config {
                    batch_size: 16,
                    generation_size: 256,
                    cpu_iterations_per_sample: 10,
                    feedback,
                    duration_seconds: 0.01,
                    evaluator_cpus: cpus[1..].to_vec(),
                    sampler_cpus: vec![cpus[0]],
                })
            })
            .join()
            .unwrap()
            .unwrap();
            assert!(result["rate"].as_f64().unwrap() > 0.);
            let completed = result["samples"].as_u64().unwrap() + 256;
            assert_eq!(
                result["sampler_diagnostics"]["returned_samples"],
                if feedback { completed } else { 0 }
            );
        }
    }
}
