//! Paired adapter/callback timings through the production Python SDK.
use crate::core::{
    AccumulatorConfig, EvaluatorConfig, SamplerAggregatorConfig, TrainingProjection,
};
use crate::{Batch, Domain, EvalBatchOptions, Generation, Point};
use anyhow::{Result, ensure};
use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256PlusPlus;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    hint::black_box,
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub python: PathBuf,
    pub fixture: PathBuf,
    pub output: PathBuf,
    pub batch_sizes: Vec<usize>,
    pub child_cpu: usize,
}
fn config(options: &Config, role: &str, domain: &Domain, feedback: bool, stats: &Path) -> Value {
    let command = vec![
        "env".to_owned(),
        "OPENBLAS_NUM_THREADS=1".into(),
        "OMP_NUM_THREADS=1".into(),
        "taskset".into(),
        "-c".into(),
        options.child_cpu.to_string(),
        options.python.display().to_string(),
        "-u".into(),
        options.fixture.display().to_string(),
        role.into(),
    ];
    let mut definition = json!({"kind":format!("process_{role}"), "command":command,
        "args":{"stats_path":stats,"feedback":feedback,"benchmark":true}});
    if role == "evaluator" {
        definition["domain"] = json!(domain);
        definition["components"] = json!(["value"]);
    } else {
        definition["requires_training_values"] = json!(feedback);
    }
    definition
}
fn accumulator() -> AccumulatorConfig {
    AccumulatorConfig::vector(vec!["value".into()], TrainingProjection::component("value"))
}
fn time_calls(mut call: impl FnMut() -> Result<()>, repetitions: usize) -> Result<Vec<f64>> {
    for _ in 0..3 {
        call()?;
    }
    (0..repetitions)
        .map(|_| {
            let start = Instant::now();
            call()?;
            Ok(start.elapsed().as_secs_f64())
        })
        .collect()
}

struct Case {
    batch: usize,
    feedback: bool,
}

fn record(
    rows: &mut Vec<Value>,
    path: &Path,
    operation: &str,
    wall: Vec<f64>,
    case: Case,
    startup: f64,
) -> Result<()> {
    let Case { batch, feedback } = case;
    let stats: Value = serde_json::from_slice(&fs::read(path)?)?;
    let callbacks = stats["timings"][operation]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing callback timings"))?;
    assert_eq!(
        callbacks.len(),
        wall.len() + 3,
        "paired callback/native call count"
    );
    let callbacks: Vec<f64> = callbacks[3..].iter().map(|v| v.as_f64().unwrap()).collect();
    assert!(
        wall.iter()
            .zip(&callbacks)
            .all(|(wall, callback)| wall >= callback)
    );
    rows.push(
        json!({"operation":operation,"batch":batch,"feedback":feedback,
        "startup_seconds":startup,"wall_seconds":wall,"callback_seconds":callbacks,
        "child_cpus":stats["cpus"],"numpy_version":stats["numpy_version"]}),
    );
    Ok(())
}

pub fn measure(options: Config) -> Result<Value> {
    ensure!(
        !options.batch_sizes.is_empty()
            && options
                .batch_sizes
                .iter()
                .all(|n| (16..=1_048_576).contains(n)),
        "invalid protocol batch sizes"
    );
    ensure!(options.fixture.is_file(), "missing Python fixture");
    let output = &options.output;
    fs::create_dir_all(output)?;
    let domain = Domain::continuous(6);
    let accumulator = accumulator();
    let mut rows = Vec::new();
    for &batch_size in &options.batch_sizes {
        let repetitions = (1_048_576 / batch_size).clamp(32, 128);
        let batch = {
            let mut rng = Xoshiro256PlusPlus::seed_from_u64(1234);
            Batch::new(
                (0..batch_size)
                    .map(|_| Point::new((0..6).map(|_| rng.random()).collect(), vec![], 2.))
                    .collect(),
            )?
        };
        for feedback in [false, true] {
            let path = output.join(format!("eval-{batch_size}-{feedback}.json"));
            let definition: EvaluatorConfig =
                serde_json::from_value(config(&options, "evaluator", &domain, feedback, &path))?;
            let start = Instant::now();
            let mut evaluator = definition.build()?;
            let startup = start.elapsed().as_secs_f64();
            let wall = time_calls(
                || {
                    // Include result cleanup, as for sampler generations below.
                    drop(black_box(evaluator.eval_batch(
                        &batch,
                        &accumulator,
                        EvalBatchOptions {
                            require_training_values: feedback,
                        },
                    )?));
                    Ok(())
                },
                repetitions,
            )?;
            drop(evaluator);
            record(
                &mut rows,
                &path,
                "eval",
                wall,
                Case {
                    batch: batch_size,
                    feedback,
                },
                startup,
            )?;
        }
        for feedback in [false, true] {
            let path = output.join(format!("sampler-{batch_size}-{feedback}.json"));
            let definition: SamplerAggregatorConfig =
                serde_json::from_value(config(&options, "sampler", &domain, feedback, &path))?;
            let start = Instant::now();
            let mut sampler = definition.build(domain.clone(), None, None, json!({}))?;
            let startup = start.elapsed().as_secs_f64();
            let produce = time_calls(
                || {
                    // Keep the native generation, including its training window,
                    // and time cleanup just as for evaluator results.
                    let generation = sampler.generate(Some(batch_size))?;
                    ensure!(
                        matches!(generation, Generation::Batch { .. }),
                        "expected a generated batch"
                    );
                    drop(black_box(generation));
                    Ok(())
                },
                repetitions,
            )?;
            let values: Vec<_> = batch.points().iter().map(|p| p.continuous[0]).collect();
            let ingest = if feedback {
                time_calls(
                    || {
                        sampler.feedback(&values)?;
                        Ok(())
                    },
                    repetitions,
                )?
            } else {
                Vec::new()
            };
            drop(sampler);
            for (operation, wall, size) in [
                ("generate", produce, batch_size),
                ("feedback", ingest, batch_size),
            ] {
                if wall.is_empty() {
                    continue;
                }
                record(
                    &mut rows,
                    &path,
                    operation,
                    wall,
                    Case {
                        batch: size,
                        feedback,
                    },
                    startup,
                )?;
            }
        }
        fs::write(
            output.join("measurements.json"),
            serde_json::to_vec_pretty(&json!({"rows":rows}))?,
        )?;
    }
    let report = json!({"schema_version":2,"experiment":"process_api","continuous_dims":6,
        "sampler_representation":"native_generation", "result_cleanup_timed":true,
        "scope":"Production Rust adapters and Python SDK; paired callback/native wall times; excludes startup, database and fleet orchestration. Includes packing, validation, IPC, native generation decoding/evaluator accumulation, result cleanup and OS scheduling delays. Sampler generations are not expanded into evaluator Points.",
        "profile_debug_assertions":cfg!(debug_assertions),"rows":rows});
    fs::write(
        output.join("measurements.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}
