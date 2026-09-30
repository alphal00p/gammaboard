use super::*;
use crate::core::SamplerAggregatorConfig;
use crate::evaluation::Materializer;
use crate::sampling::{IdentityMaterializer, NaiveMonteCarloSamplerParams};

#[test]
fn legacy_state_is_rejected_without_modifying_its_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("state_manifest.toml");
    std::fs::write(&manifest, "version = 7\n").unwrap();
    let error = GammaLoopEvaluator::from_params(GammaLoopParams {
        state_folder: temp.path().to_path_buf(),
        ..Default::default()
    })
    .err()
    .unwrap();
    assert!(
        error.to_string().contains("regenerate the saved state"),
        "{error}"
    );
    assert_eq!(std::fs::read_to_string(manifest).unwrap(), "version = 7\n");
}

#[test]
fn read_only_preprocessing_rejects_saving_into_the_active_state() {
    let temp = tempfile::tempdir().unwrap();
    let state_folder = temp.path().join("state");
    let params = GammaLoopParams {
        state_folder: state_folder.clone(),
        preprocessing: GammaLoopPreprocessing {
            commands: vec!["save state".to_string()],
            read_only: true,
        },
        ..GammaLoopParams::default()
    };
    let mut state = State::new_test();
    let err = GammaLoopEvaluator::run_preprocessing(&params, &mut state).unwrap_err();
    assert!(err.to_string().contains("--read-only-state"), "{err}");
    assert!(!state_folder.exists());
}

fn stable_result(result: &BatchResult) -> JsonValue {
    fn remove_timings(value: &mut JsonValue) {
        match value {
            JsonValue::Object(map) => {
                map.retain(|key, _| !key.ends_with("_time_ms"));
                map.values_mut().for_each(remove_timings);
            }
            JsonValue::Array(items) => items.iter_mut().for_each(remove_timings),
            _ => {}
        }
    }
    let mut value = serde_json::to_value(result).unwrap();
    remove_timings(&mut value);
    value
}

#[test]
#[ignore = "requires GAMMABOARD_TEST_PHYSICS_STATE and GAMMABOARD_TEST_PHYSICS_INTEGRAND with histograms"]
fn observable_batches_are_isolated_after_mixed_modes_and_failure() {
    let mut evaluator = GammaLoopEvaluator::from_params(GammaLoopParams {
        state_folder: std::env::var("GAMMABOARD_TEST_PHYSICS_STATE")
            .expect("read-only generated physics state")
            .into(),
        integrand_name: Some(
            std::env::var("GAMMABOARD_TEST_PHYSICS_INTEGRAND").expect("integrand with histograms"),
        ),
        preprocessing: GammaLoopPreprocessing {
            read_only: true,
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    let mut sampler = SamplerAggregatorConfig::NaiveMonteCarlo {
        params: NaiveMonteCarloSamplerParams {
            seed: 1234,
            ..Default::default()
        },
        materializer: None,
    }
    .build(evaluator.get_domain(), None, None, evaluator.metadata())
    .unwrap();
    let options = EvalBatchOptions {
        require_training_values: true,
    };
    let modes = [
        AccumulatorConfig::Gammaloop,
        AccumulatorConfig::scalar(),
        AccumulatorConfig::vector(
            vec!["re".into(), "im".into()],
            crate::core::TrainingProjection::Component { name: "re".into() },
        ),
        AccumulatorConfig::Empty,
    ];
    for size in [1, 32, 7] {
        let latent = sampler
            .generate(Some(size))
            .and_then(|generated| generated.into_batch())
            .unwrap()
            .with_accumulator_config(AccumulatorConfig::Gammaloop)
            .build();
        let batch = IdentityMaterializer::new()
            .materialize_batch(&latent)
            .unwrap();
        let expected = modes.each_ref().map(|mode| {
            evaluator.integrand = evaluator.pristine_integrand.clone();
            stable_result(&evaluator.eval_batch(&batch, mode, options).unwrap())
        });
        let snapshot = evaluator
            .eval_batch(&batch, &AccumulatorConfig::Gammaloop, options)
            .unwrap();
        let AccumulatorState::Gammaloop(state) = &snapshot.accumulator else {
            panic!("expected GammaLoop observables")
        };
        assert!(state.histogram_count() > 0);
        assert_eq!(state.diagnostics.count_total, size as i64);

        for (mode, expected_mode) in modes.iter().zip(&expected) {
            let actual = evaluator.eval_batch(&batch, mode, options).unwrap();
            assert_eq!(stable_result(&actual), *expected_mode);
            let actual = evaluator
                .eval_batch(&batch, &AccumulatorConfig::Gammaloop, options)
                .unwrap();
            assert_eq!(stable_result(&actual), expected[0]);
        }

        // Bypass GammaBoard's input validation to exercise an external failure
        // after one valid sample has already updated the integrand's observables.
        let point = &batch.points()[0];
        let discrete = point
            .discrete
            .iter()
            .map(|&x| x as usize)
            .collect::<Vec<_>>();
        let samples = [
            havana_sample(
                point.continuous.iter().copied().map(F).collect(),
                &discrete,
                F(1.0),
            ),
            Sample::Continuous(F(1.0), vec![]),
        ];
        let failure = GammaLoopEvaluator::call_external("evaluate_samples_raw", || {
            evaluator.integrand.evaluate_samples_raw(
                EvaluationTarget::Physical(&evaluator.model),
                &samples,
                1,
                false,
                false,
                Default::default(),
            )
        });
        assert!(failure.is_err());
        let recovered = evaluator
            .eval_batch(&batch, &AccumulatorConfig::Gammaloop, options)
            .unwrap();
        assert_eq!(stable_result(&recovered), expected[0]);
        assert_eq!(stable_result(&snapshot), expected[0]);
    }
}

#[test]
fn gammaloop_efficiency_uses_weighted_training_projection_through_merge_and_roundtrip() {
    use crate::evaluation::{Accumulator, ScalarAccumulatorState};
    let mut results = vec![EvaluationResult::zero(), EvaluationResult::zero()];
    results[0].integrand_result.re = F(3.0);
    results[0].integrand_result.im = F(4.0);
    results[0].parameterization_jacobian = Some(F(2.0));
    results[1].integrand_result.re = F(-12.0);
    results[1].integrand_result.im = F(5.0);
    results[1].parameterization_jacobian = Some(F(0.5));
    let points = vec![
        crate::Point::new(vec![], vec![], 3.0),
        crate::Point::new(vec![], vec![], 2.0),
    ];
    for (projection, expected) in [
        (TrainingProjection::Real, [18.0, -12.0]),
        (TrainingProjection::Imag, [24.0, 5.0]),
        (TrainingProjection::Abs, [30.0, 13.0]),
        // Parameterization is squared; the Monte Carlo weight is applied once.
        (TrainingProjection::AbsSq, [300.0, 84.5]),
    ] {
        let whole = GammaLoopEvaluator::gammaloop_estimate(&results, &points, projection);
        let mut merged = GammaLoopAccumulatorState::default();
        for i in 0..2 {
            merged
                .merge_in_place(GammaLoopEvaluator::gammaloop_estimate(
                    &results[i..i + 1],
                    &points[i..i + 1],
                    projection,
                ))
                .unwrap();
        }
        assert_eq!(
            whole.to_persistent_json().unwrap(),
            merged.to_persistent_json().unwrap()
        );
        let restored: GammaLoopAccumulatorState =
            serde_json::from_value(merged.to_persistent_json().unwrap()).unwrap();
        let mut reference = ScalarAccumulatorState::plain();
        for value in expected {
            reference.add_sample(value, &crate::Point::new(vec![], vec![], 1.0));
        }
        assert_eq!(
            restored.training_statistics().unwrap().mean(),
            reference.mean()
        );
        assert_eq!(restored.rsd(), Some(reference.rsd()));
        assert_eq!(restored.ess(), Some(reference.ess()));
        assert_eq!(restored.norm_statistics().mean(), 21.5);
        assert_eq!(restored.real_mean(), 3.0);
        assert_eq!(restored.imag_mean(), 14.5);
        let positive_real_point = restored
            .estimate
            .component("real")
            .unwrap()
            .state
            .max_weighted_positive_point
            .as_ref()
            .unwrap();
        assert_eq!(positive_real_point.integrand_value_re, Some(3.0));
        assert_eq!(positive_real_point.integrand_value_im, Some(4.0));
        assert_eq!(positive_real_point.parameterization_jacobian, Some(2.0));
        assert_eq!(
            positive_real_point.factor_value("sampler_weight"),
            Some(3.0)
        );
        for (name, expected_metric) in [("rsd", reference.rsd()), ("ess", reference.ess())] {
            let selector = serde_json::from_value(json!({"name":name})).unwrap();
            let metric = crate::evaluation::extract_accumulator_metric(
                &AccumulatorState::Gammaloop(restored.clone()),
                &selector,
            )
            .unwrap()
            .unwrap();
            assert_eq!(metric.value, expected_metric);
        }

        // Upgrading an old accumulator must use its existing phase moments,
        // and must not silently label partial AbsSq statistics as complete.
        let mut old = GammaLoopEvaluator::gammaloop_estimate(
            &results[..1],
            &points[..1],
            TrainingProjection::Abs,
        );
        old.training_projection = None;
        old.merge_in_place(GammaLoopEvaluator::gammaloop_estimate(
            &results[1..],
            &points[1..],
            projection,
        ))
        .unwrap();
        if projection == TrainingProjection::AbsSq {
            assert!(old.training_statistics().is_none());
        } else {
            assert_eq!(old.rsd(), restored.rsd());
            assert_eq!(old.ess(), restored.ess());
        }
    }
}
