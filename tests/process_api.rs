//! Real Rust adapters and the Python SDK, without PostgreSQL or a worker fleet.
use gammaboard::core::{AccumulatorConfig, EvaluatorConfig, SamplingConfig, TrainingProjection};
use gammaboard::{Batch, Domain, EvalBatchOptions, Point};
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
        definition["output"] = json!({"kind":"values", "components":["value"]});
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
    let sampler: SamplingConfig = serde_json::from_value(config(
        "sampler",
        &domain,
        0,
        &directory.path().join("sampler.json"),
    ))?;
    let mut evaluator = evaluator.build()?;
    let metadata = evaluator.metadata();
    assert_eq!(metadata, json!({"space":"coordinates", "dims":6}));
    let mut sampler = sampler.create(domain, None, metadata.clone())?;
    assert_eq!(sampler.snapshot()?["evaluator_metadata"], metadata);
    let mut count = 0;
    let mut total = 0.;
    for size in [1, 17, 4096] {
        let latent = sampler
            .generate(size)
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
                stratification: None,
            },
        )?;
        let values = result.training_values.unwrap();
        let expected: Vec<_> = (0..size).map(|i| (0.5 + (i % 3) as f64) * 2.).collect();
        assert_eq!(
            values, expected,
            "feedback must include the sampling weight exactly once"
        );
        let collected = evaluator.eval_batch(
            &batch,
            &AccumulatorConfig::FullVector {
                components: vec!["value".into()],
            },
            EvalBatchOptions {
                require_training_values: false,
                stratification: None,
            },
        )?;
        let gammaboard::AccumulatorState::FullVector(collected) = collected.accumulator else {
            panic!("full vector result");
        };
        assert_eq!(collected.values_row_major, expected);
        sampler.feedback(&values)?;
        count += size;
        total += values.iter().sum::<f64>();
        assert_eq!(sampler.diagnostics(), json!({"count":count,"total":total}));
        assert_eq!(sampler.global_pdf_norm()?, 2.5);
        assert_eq!(
            sampler.persisted_output()?,
            Some(json!({"count":count,"total":total}))
        );
        assert_eq!(evaluator.diagnostics(), json!({"count":0,"total":0.0}));
        assert!(
            evaluator
                .eval_batch(
                    &batch,
                    &accumulator(),
                    EvalBatchOptions {
                        require_training_values: false,
                        stratification: None
                    }
                )?
                .training_values
                .is_none()
        );
    }
    assert!(sampler.generate(0).is_err());
    let bad = Batch::new(vec![Point::new(vec![f64::NAN; 6], vec![0], 1.)]);
    assert!(
        evaluator
            .eval_batch(
                &bad,
                &accumulator(),
                EvalBatchOptions {
                    require_training_values: false,
                    stratification: None
                }
            )
            .is_err()
    );
    let good = Batch::new(vec![Point::new(vec![0.5; 6], vec![0], 1.)]);
    assert!(
        evaluator
            .eval_batch(
                &good,
                &accumulator(),
                EvalBatchOptions {
                    require_training_values: false,
                    stratification: None
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

#[test]
#[cfg(feature = "gammaloop")]
#[ignore = "requires Python and NumPy; set GAMMABOARD_PROCESS_PYTHON"]
fn gammaloop_process_preserves_weights_and_feedback_options() -> Result {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config: EvaluatorConfig = serde_json::from_value(json!({
        "kind": "process_evaluator", "output": {"kind": "gammaloop"}, "domain": Domain::continuous(1),
        "cwd": root,
        "command": ["env", format!("PYTHONPATH={}:{}", root.join("process_api/python/src").display(),
            root.join("process_api/examples/python_gammaloop_observable/src").display()),
            env::var("GAMMABOARD_PROCESS_PYTHON").unwrap_or_else(|_| "python3".into()),
            "-c", "from demo_evaluator import PolynomialEvaluator; from gammaboard_process import run_evaluator; run_evaluator(PolynomialEvaluator)"]
    }))?;
    let mut evaluator = config.build()?;
    let batch = Batch::new(vec![
        Point::new(vec![0.25], vec![], 2.),
        Point::new(vec![0.75], vec![], 3.),
    ]);
    for required in [true, false] {
        let result = evaluator.eval_batch(
            &batch,
            &AccumulatorConfig::Gammaloop,
            EvalBatchOptions {
                require_training_values: required,
                stratification: None,
            },
        )?;
        let gammaboard::AccumulatorState::Gammaloop(state) = result.accumulator else {
            panic!("GammaLoop state")
        };
        assert_eq!(state.estimate.components[0].state.sum_weighted_value, 5.625);
        assert_eq!(result.training_values, required.then(|| vec![2.25, 3.375]));
    }
    let plan = gammaboard::core::StratificationPlan {
        proposal_id: "process-hybrid".into(),
        partition: gammaboard::core::StratificationConfig::Voronoi {
            centers: vec![vec![0., 0.], vec![1.5, 0.]],
            scales: None,
        },
        probabilities: Some(vec![0.25, 0.75]),
        components: vec!["real".into(), "imag".into()],
    };
    let labels: Vec<_> = (0..128).map(|i| (i / 3) % 2).collect();
    let batch = Batch::new(
        labels
            .iter()
            .enumerate()
            .map(|(i, &k)| {
                Point::new(
                    vec![[0.1, 0.5, 0.9][i % 3]],
                    vec![],
                    1. / plan.probabilities.as_ref().unwrap()[k],
                )
            })
            .collect(),
    );
    let result = evaluator.eval_batch(
        &batch,
        &AccumulatorConfig::Gammaloop,
        EvalBatchOptions {
            require_training_values: true,
            stratification: Some((&plan, &labels)),
        },
    )?;
    let gammaboard::AccumulatorState::Stratified(state) = result.accumulator else {
        panic!("hybrid state")
    };
    let pooled = state.pooled.as_ref().unwrap();
    let histogram = &pooled.bundle.histograms["x"];
    let mut sum = 0.;
    let mut sums = [0.; 20];
    let mut squares = [0.; 20];
    let mut conditional = [0.; 2];
    let mut counts = [0; 2];
    for (i, (&k, point)) in labels.iter().zip(batch.points()).enumerate() {
        let x = point.continuous[0];
        let f = 6. * x * (1. - x);
        let hit = (f > 0.75) == (k == 1);
        let contribution = if hit { point.total_weight() * f } else { 0. };
        assert_eq!(result.training_values.as_ref().unwrap()[i], contribution);
        sum += contribution;
        let b = (x * 20.) as usize;
        sums[b] += contribution;
        squares[b] += contribution * contribution;
        conditional[k] += contribution * plan.probabilities.as_ref().unwrap()[k];
        counts[k] += 1;
    }
    assert_eq!(histogram.sample_count, 128);
    assert!((pooled.real_mean() - sum / 128.).abs() < 1e-12);
    for (i, b) in histogram.bins.iter().enumerate() {
        assert!((b.sum_weights - sums[i]).abs() < 1e-12);
        assert!((b.sum_weights_squared - squares[i]).abs() < 1e-12);
    }
    assert!(
        (state.mean().unwrap()[0]
            - conditional[0] / counts[0] as f64
            - conditional[1] / counts[1] as f64)
            .abs()
            < 1e-12
    );
    assert!(state.covariance().is_some());
    Ok(())
}
