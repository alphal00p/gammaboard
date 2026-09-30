//! Real Rust adapters and the Python SDK, without PostgreSQL or a worker fleet.
use gammaboard::core::{
    AccumulatorConfig, EvaluatorConfig, SamplerAggregatorConfig, TrainingProjection,
};
use gammaboard::{Batch, Domain, EvalBatchOptions, Generation, Point};
use serde_json::{Value, json};
use std::{env, error::Error, fs, hint::black_box, path::Path, time::Instant};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

fn config(role: &str, domain: &Domain, work: usize, stats: &Path) -> Value {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut command = vec![
        "env".to_owned(),
        "OPENBLAS_NUM_THREADS=1".into(),
        "OMP_NUM_THREADS=1".into(),
    ];
    if let Ok(cpu) = env::var("GAMMABOARD_PROCESS_CHILD_CPU") {
        command.extend(["taskset".into(), "-c".into(), cpu]);
    }
    command.extend([
        env::var("GAMMABOARD_PROCESS_PYTHON").unwrap_or_else(|_| "python3".into()),
        "-u".into(),
        root.join("process_api/python/tests/runtime_fixture.py")
            .display()
            .to_string(),
        role.into(),
    ]);
    let mut definition = json!({"kind":format!("process_{role}"), "command":command,
           "cwd":root, "args":{"work":work,"stats_path":stats}});
    if role == "evaluator" {
        definition["domain"] = json!(domain);
        definition["components"] = json!(["value"]);
    } else {
        definition["requires_training_values"] = json!(true);
    }
    definition
}

fn accumulator() -> AccumulatorConfig {
    AccumulatorConfig::vector(vec!["value".into()], TrainingProjection::component("value"))
}

#[test]
#[ignore = "requires Python and NumPy; set GAMMABOARD_PROCESS_PYTHON"]
fn process_api_roundtrips() -> Result {
    let directory = tempfile::tempdir()?;
    let domain = Domain::rectangular_with_cardinalities(6, [3]);
    let evaluator: EvaluatorConfig = serde_json::from_value(config(
        "evaluator",
        &domain,
        0,
        &directory.path().join("eval.json"),
    ))?;
    let sampler: SamplerAggregatorConfig = serde_json::from_value(config(
        "sampler",
        &domain,
        0,
        &directory.path().join("sampler.json"),
    ))?;
    let mut evaluator = evaluator.build()?;
    let mut sampler = sampler.build(domain, None, None, json!({}))?;
    let mut count = 0;
    let mut total = 0.;
    for size in [1, 17, 4096] {
        let latent = sampler
            .generate(Some(size))
            .and_then(|generated| generated.into_batch())?;
        assert_eq!(latent.nr_samples, size);
        let batch = latent.payload.into_batch()?;
        for (i, point) in batch.points().iter().enumerate() {
            assert_eq!(point.continuous, vec![0.5; 6]);
            assert_eq!(point.discrete, vec![(i % 3) as i64]);
            assert_eq!(point.total_weight(), 2.);
        }
        let result = evaluator.eval_batch(
            &batch,
            &accumulator(),
            EvalBatchOptions {
                require_training_values: true,
            },
        )?;
        let values = result.values.unwrap();
        let expected: Vec<_> = (0..size).map(|i| (0.5 + (i % 3) as f64) * 2.).collect();
        assert_eq!(
            values, expected,
            "feedback must include the sampling weight exactly once"
        );
        sampler.feedback(&values)?;
        count += size;
        total += values.iter().sum::<f64>();
        assert_eq!(
            sampler.get_diagnostics(),
            json!({"count":count,"total":total})
        );
        assert!(
            evaluator
                .eval_batch(
                    &batch,
                    &accumulator(),
                    EvalBatchOptions {
                        require_training_values: false
                    }
                )?
                .values
                .is_none()
        );
    }
    assert!(matches!(sampler.generate(Some(0))?, Generation::Finished));
    let bad = Batch::new(vec![Point::new(vec![f64::NAN; 6], vec![0], 1.)])?;
    assert!(
        evaluator
            .eval_batch(
                &bad,
                &accumulator(),
                EvalBatchOptions {
                    require_training_values: false
                }
            )
            .is_err()
    );
    let good = Batch::new(vec![Point::new(vec![0.5; 6], vec![0], 1.)])?;
    assert!(
        evaluator
            .eval_batch(
                &good,
                &accumulator(),
                EvalBatchOptions {
                    require_training_values: false
                }
            )
            .is_ok(),
        "worker survives a rejected request"
    );
    drop((evaluator, sampler));
    for role in ["eval", "sampler"] {
        let stats: Value =
            serde_json::from_slice(&fs::read(directory.path().join(format!("{role}.json")))?)?;
        assert!(
            stats["timings"].is_object(),
            "graceful shutdown must flush callback evidence"
        );
    }
    Ok(())
}

fn time_calls(mut call: impl FnMut() -> Result, repetitions: usize) -> Result<Vec<f64>> {
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
    work: usize,
    feedback: bool,
}

fn record(
    rows: &mut Vec<Value>,
    path: &Path,
    operation: &str,
    wall: Vec<f64>,
    case: Case,
    startup: f64,
) -> Result {
    let Case {
        batch,
        work,
        feedback,
    } = case;
    let stats: Value = serde_json::from_slice(&fs::read(path)?)?;
    let callbacks = stats["timings"][operation]
        .as_array()
        .ok_or("missing callback timings")?;
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
        json!({"operation":operation,"batch":batch,"work":work,"feedback":feedback,
        "startup_seconds":startup,"wall_seconds":wall,"callback_seconds":callbacks,
        "child_cpus":stats["cpus"],"numpy_version":stats["numpy_version"]}),
    );
    Ok(())
}

#[test]
#[ignore = "targeted overhead measurements; set GAMMABOARD_PROCESS_OUTPUT and use an optimized profile"]
fn process_api_overhead() -> Result {
    let output = env::var("GAMMABOARD_PROCESS_OUTPUT")?;
    let output = Path::new(&output);
    fs::create_dir(output)?;
    let domain = Domain::continuous(6);
    let accumulator = accumulator();
    let mut rows = Vec::new();
    for work in [0, 64] {
        for (batch_size, repetitions) in [
            (16, 128),
            (64, 128),
            (256, 128),
            (1024, 64),
            (4096, 32),
            (16384, 32),
            (65536, 16),
            (262144, 8),
            (1048576, 4),
        ] {
            let batch = Batch::new(vec![Point::new(vec![0.5; 6], vec![], 2.); batch_size])?;
            for feedback in [false, true] {
                let path = output.join(format!("eval-{work}-{batch_size}-{feedback}.json"));
                let definition: EvaluatorConfig =
                    serde_json::from_value(config("evaluator", &domain, work, &path))?;
                let start = Instant::now();
                let mut evaluator = definition.build()?;
                let startup = start.elapsed().as_secs_f64();
                let wall = time_calls(
                    || {
                        black_box(evaluator.eval_batch(
                            &batch,
                            &accumulator,
                            EvalBatchOptions {
                                require_training_values: feedback,
                            },
                        )?);
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
                        work,
                        feedback,
                    },
                    startup,
                )?;
            }
            let path = output.join(format!("sampler-{work}-{batch_size}.json"));
            let definition: SamplerAggregatorConfig =
                serde_json::from_value(config("sampler", &domain, work, &path))?;
            let start = Instant::now();
            let mut sampler = definition.build(domain.clone(), None, None, json!({}))?;
            let startup = start.elapsed().as_secs_f64();
            let produce = time_calls(
                || {
                    black_box(
                        sampler
                            .generate(Some(batch_size))
                            .and_then(|generated| generated.into_batch())?,
                    );
                    Ok(())
                },
                repetitions,
            )?;
            let values = vec![0.5; batch_size];
            let ingest = time_calls(
                || {
                    sampler.feedback(&values)?;
                    Ok(())
                },
                repetitions,
            )?;
            drop(sampler);
            for (operation, wall, size) in [
                ("generate", produce, batch_size),
                ("feedback", ingest, batch_size),
            ] {
                record(
                    &mut rows,
                    &path,
                    operation,
                    wall,
                    Case {
                        batch: size,
                        work,
                        feedback: true,
                    },
                    startup,
                )?;
            }
        }
    }
    let report = json!({"schema_version":1,"experiment":"process_api","continuous_dims":6,
        "scope":"Production Rust adapters and Python SDK; paired callback/native wall times; excludes startup, database and fleet orchestration. Includes packing, validation, IPC, native accumulation/conversion and OS scheduling delays.",
        "profile_debug_assertions":cfg!(debug_assertions),"rows":rows});
    fs::write(
        output.join("measurements.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}
