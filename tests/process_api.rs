//! Real Rust adapters and the Python SDK, without PostgreSQL or a worker fleet.
use gammaboard::core::{
    AccumulatorConfig, EvaluatorConfig, SamplerAggregatorConfig, TrainingProjection,
};
use gammaboard::{Batch, Domain, EvalBatchOptions, Generation, Point};
use serde_json::{Value, json};
use std::{env, error::Error, fs, path::Path};

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
