//! Acceptance tests using a separately built wrapper and an explicit saved state.
//! Also run values_roundtrip with --no-default-features to exercise a GammaLoop-free host.
use gammaboard::{
    AccumulatorConfig, Batch, Domain, EvalBatchOptions, EvaluatorBatch, EvaluatorConfig, Point,
};
use serde_json::{Value, json};
use std::{env, fs, process::Command};

fn setup(output: Value) -> anyhow::Result<(EvaluatorConfig, Value, Domain)> {
    let binary = env::var("GAMMABOARD_TEST_GAMMALOOP_PROCESS")?;
    let args_file = env::var("GAMMABOARD_TEST_GAMMALOOP_ARGS")?;
    let args: Value = serde_json::from_slice(&fs::read(&args_file)?)?;
    let description = Command::new(&binary)
        .args(["--describe", &args_file])
        .output()?;
    anyhow::ensure!(
        description.status.success(),
        "{}",
        String::from_utf8_lossy(&description.stderr)
    );
    // The description must be clean JSON even when the native libraries print.
    let description: Value = serde_json::from_slice(&description.stdout)?;
    let domain: Domain = serde_json::from_value(description["domain"].clone())?;
    let config = serde_json::from_value(json!({
        "kind":"process_evaluator", "command":[binary], "domain":domain,
        "output":output, "args":args,
    }))?;
    Ok((config, args, domain))
}

fn points(domain: &Domain, prefix: Vec<i64>, result: &mut Vec<Point>) {
    match domain {
        Domain::Discrete { branches, .. } => {
            for branch in branches.iter().take(3) {
                let mut path = prefix.clone();
                path.push(branch.index as i64);
                points(&branch.domain, path, result);
            }
        }
        Domain::Rectangular {
            continuous_dims,
            discrete_cardinalities,
            ..
        } => {
            let mut path = prefix;
            path.extend(vec![0; discrete_cardinalities.len()]);
            for i in 0..7 {
                let xs = (0..*continuous_dims)
                    .map(|j| 0.13 + ((i * 7 + j * 3) % 19) as f64 / 25.)
                    .collect();
                result.push(Point::new(xs, path.clone(), 2.5));
            }
        }
    }
}

fn batch(domain: &Domain) -> Batch {
    let mut samples = vec![];
    points(domain, vec![], &mut samples);
    Batch::new(samples)
}

fn validate_card(source: &str, evaluator: &EvaluatorConfig) -> anyhow::Result<()> {
    let mut card: toml::Value = toml::from_str(source)?;
    card["evaluator"] = toml::Value::try_from(evaluator)?;
    let definition = gammaboard::api::runs::parse_run_add_config_toml(&toml::to_string(&card)?)?;
    gammaboard::api::runs::validate_run(definition, true)?;
    Ok(())
}

#[test]
#[ignore = "requires GAMMABOARD_TEST_GAMMALOOP_PROCESS and GAMMABOARD_TEST_GAMMALOOP_ARGS"]
fn values_roundtrip() -> anyhow::Result<()> {
    let components = vec!["real".into(), "imag".into()];
    let (config, _, domain) = setup(json!({"kind":"values", "components":components}))?;
    validate_card(
        include_str!("../process_api/examples/rust_gammaloop/run.toml"),
        &config,
    )?;
    let mut evaluator = config.build()?;
    assert_eq!(evaluator.metadata()["kind"], "gammaloop");
    let batch = batch(&domain);
    let raw = evaluator.eval_values(&batch, &components)?;
    assert_eq!(raw.len(), batch.len() * 2);
    assert!(raw.iter().all(|v| v.is_finite()));
    assert!(
        raw.iter().any(|v| *v != 0.0),
        "fixture must exercise a nonzero integrand"
    );
    assert_eq!(raw, evaluator.eval_values(&batch, &components)?);
    assert!(
        evaluator
            .eval_values(&Batch::new(vec![]), &components)?
            .is_empty()
    );

    let result = evaluator.eval_batch(
        &batch,
        &AccumulatorConfig::FullVector {
            components: components.clone(),
        },
        EvalBatchOptions::default(),
    )?;
    let gammaboard::AccumulatorState::FullVector(result) = result.accumulator else {
        panic!("full vector");
    };
    assert_eq!(
        result.values_row_major,
        raw.iter().map(|v| v * 2.5).collect::<Vec<_>>()
    );
    let mut bad = batch.points().to_vec();
    bad[0].continuous[0] = f64::NAN;
    assert!(
        evaluator
            .eval_values(&Batch::new(bad), &components)
            .is_err()
    );
    assert_eq!(
        raw,
        evaluator.eval_values(&batch, &components)?,
        "bad request must not corrupt subsequent evaluation"
    );
    drop(evaluator);
    let mut wrong_domain = serde_json::to_value(config)?;
    wrong_domain["domain"] = json!(Domain::continuous(999));
    let error = serde_json::from_value::<EvaluatorConfig>(wrong_domain)?
        .build()
        .err()
        .expect("initialization must reject a mismatched domain");
    assert!(
        error.to_string().contains("configured domain differs"),
        "{error}"
    );
    Ok(())
}

#[cfg(feature = "gammaloop")]
#[test]
#[ignore = "requires GAMMABOARD_TEST_GAMMALOOP_PROCESS and GAMMABOARD_TEST_GAMMALOOP_ARGS"]
fn native_and_process_agree() -> anyhow::Result<()> {
    let (config, args, domain) = setup(json!({"kind":"values", "components":["real","imag"]}))?;
    let native_config = EvaluatorConfig::Gammaloop {
        params: serde_json::from_value(args)?,
    };
    validate_card(
        include_str!("../process_api/examples/rust_gammaloop/run-native.toml"),
        &native_config,
    )?;
    let mut native = native_config.build()?;
    let mut process = config.build()?;
    assert_eq!(native.metadata(), process.metadata());
    let batch = batch(&domain);
    let components = vec!["real".into(), "imag".into()];
    assert_eq!(
        native.eval_values(&batch, &components)?,
        process.eval_values(&batch, &components)?
    );
    drop(process);

    let mut config = serde_json::to_value(config)?;
    config["output"] = json!({"kind":"values","components":["value"]});
    let mut scalar = serde_json::from_value::<EvaluatorConfig>(config.clone())?.build()?;
    assert_eq!(
        native.eval_values(&batch, &["value".into()])?,
        scalar.eval_values(&batch, &["value".into()])?
    );
    drop(scalar);

    config["output"] = json!({"kind":"gammaloop", "training_projection":"abs"});
    for format in ["binary", "json"] {
        let mut selected = config.clone();
        selected["command"]
            .as_array_mut()
            .unwrap()
            .extend([json!("--observable-format"), json!(format)]);
        let mut process = serde_json::from_value::<EvaluatorConfig>(selected)?.build()?;
        // Empty batches before and after schema discovery must be valid.
        process.eval_samples(&Batch::new(vec![]), &components, &mut |_| {
            panic!("empty batch emitted a sample")
        })?;
        let mut expected = Vec::new();
        native.eval_samples(&batch, &components, &mut |sample| {
            expected.push((
                sample.values,
                serde_json::to_value(sample.observables).unwrap(),
            ));
            Ok(())
        })?;
        let mut actual = Vec::new();
        process.eval_samples(&batch, &components, &mut |sample| {
            actual.push((
                sample.values,
                serde_json::to_value(sample.observables).unwrap(),
            ));
            Ok(())
        })?;
        assert_eq!(expected, actual);

        // The host reduces unweighted bundles and applies outer weights exactly once.
        let options = EvalBatchOptions::default();
        let expected = native.eval_batch(&batch, &AccumulatorConfig::Gammaloop, options)?;
        let actual = process.eval_batch(&batch, &AccumulatorConfig::Gammaloop, options)?;
        let (
            gammaboard::AccumulatorState::Gammaloop(expected),
            gammaboard::AccumulatorState::Gammaloop(actual),
        ) = (expected.accumulator, actual.accumulator)
        else {
            panic!("observable accumulators");
        };
        assert_eq!(
            serde_json::to_value(expected.bundle)?,
            serde_json::to_value(actual.bundle)?
        );
    }
    Ok(())
}

#[cfg(feature = "gammaloop")]
#[test]
#[ignore = "requires GAMMABOARD_TEST_GAMMALOOP_PROCESS and GAMMABOARD_TEST_GAMMALOOP_ARGS"]
fn discovered_contract_supports_mixed_requests_and_full_details() -> anyhow::Result<()> {
    let (config, args, domain) = setup(json!({"kind":"values","components":["real","imag"]}))?;
    let mut value = serde_json::to_value(config)?;
    value.as_object_mut().unwrap().remove("domain");
    value.as_object_mut().unwrap().remove("output");
    let mut config: EvaluatorConfig = serde_json::from_value(value)?;
    let descriptor = config.discover()?;
    assert_eq!(descriptor.domain, domain);
    assert!(descriptor.observables && descriptor.details && descriptor.diagnostics);
    let native_config = EvaluatorConfig::Gammaloop {
        params: serde_json::from_value(args)?,
    };
    let mut native = native_config.build()?;
    let batch = batch(&domain);
    for format in ["binary", "json"] {
        let mut selected = serde_json::to_value(&config)?;
        selected["command"]
            .as_array_mut()
            .unwrap()
            .extend([json!("--observable-format"), json!(format)]);
        let mut process = serde_json::from_value::<EvaluatorConfig>(selected)?.build()?;
        // Exercise changing order/subsets and returning to observables after a values-only call.
        for (components, observables, details, diagnostics) in [
            (vec!["imag", "real"], true, true, true),
            (vec!["value"], false, false, false),
            (vec!["imag"], false, true, true),
            (vec!["real", "imag"], true, true, true),
        ] {
            let request = gammaboard::EvaluationRequest {
                components: components.into_iter().map(str::to_owned).collect(),
                observables,
                details,
                diagnostics,
            };
            let collect =
                |evaluator: &mut dyn gammaboard::Evaluator| -> anyhow::Result<Vec<Value>> {
                    let mut rows = Vec::new();
                    gammaboard::evaluate_requested(evaluator, &batch, &request, &mut |sample| {
                        let mut row = serde_json::to_value(sample).unwrap();
                        if diagnostics {
                            let d = row["diagnostics"].as_object_mut().unwrap();
                            assert_eq!(d["count_total"], 1);
                            for (name, value) in d.iter() {
                                if name.ends_with("_ms") && !value.is_null() {
                                    assert!(value.as_f64().unwrap() >= 0.);
                                }
                            }
                            // Wall-clock timings are measured separately, never numerically equal.
                            d.retain(|name, _| !name.ends_with("_ms"));
                        }
                        rows.push(row);
                        Ok(())
                    })?;
                    Ok(rows)
                };
            assert_eq!(collect(native.as_mut())?, collect(process.as_mut())?);
        }
    }
    Ok(())
}
