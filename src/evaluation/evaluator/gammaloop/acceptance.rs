//! Saved-state acceptance exercises the real GammaLoop maps and observable runtime.
use super::*;
use crate::evaluation::{Accumulator, Point};
use gammaloop_api::state::GraphImportOptions;
use gammalooprs::{
    graph::Graph, model::InputParamCard, settings::GlobalSettings, utils::load_generic_model,
};
use std::sync::OnceLock;

fn fixture() -> &'static std::path::Path {
    static STATE: OnceLock<tempfile::TempDir> = OnceLock::new();
    STATE
        .get_or_init(|| {
            crate::activate_symbolica_oem_license().unwrap();
            initialise().unwrap();
            let temp = tempfile::tempdir().unwrap();
            let mut state = State::new_test();
            state.model = load_generic_model("scalars");
            state.model_parameters = InputParamCard::default_from_model(&state.model);
            state
                .import_graphs(
                    Graph::from_string(
                        r#"
            digraph sampling_cut_bubble {
                num=1; edge [pdg=1001]; node [num=1];
                ext [style=invis, is_cut=0];
                ext -> a [id=0];
                a -> b [id=1, lmb_id=0];
                a -> b [id=2];
                b -> ext [id=3];
            }
        "#,
                        &state.model,
                    )
                    .unwrap(),
                    GraphImportOptions {
                        process_name: Some("acceptance".into()),
                        process_id: None,
                        process_definition: None,
                        integrand_name: Some("default".into()),
                        overwrite: false,
                        append: false,
                    },
                )
                .unwrap();
            let global: GlobalSettings = toml::from_str(
                r#"
            [n_cores]
            generate = 1
            compile = 1
            [generation]
            override_lmb_heuristics = true
            [generation.uv]
            subtract_uv = false
            generate_integrated = false
            [generation.evaluator]
            compile = false
            summed = false
            summed_function_map = true
            iterative_orientation_optimization = false
        "#,
            )
            .unwrap();
            let runtime: RuntimeSettings = toml::from_str(
                r#"
            [general]
            evaluator_method = "SummedFunctionMap"
            integral_unit = "none"
            enable_cache = false
            [kinematics]
            e_cm = 5.0
            [kinematics.externals]
            type = "constant"
            [kinematics.externals.data]
            momenta = [[5.0, 0.0, 0.0, 0.0]]
            helicities = ["summed_averaged"]
            [sampling]
            graphs = "summed"
            orientations = "summed"
            sampling_multichanneling = true
            sampling_channels = "summed"
            sampling_channel_weight = "map_density"
            default_channel_selection = ["ordinary", "cut"]
            [sampling.channel_definitions.sampling_cut_bubble.ordinary]
            around = "lmb(1)"
            parent_lmb = [1]
            [sampling.channel_definitions.sampling_cut_bubble.cut]
            around = "phase_space(cut(1,2))"
            parent_lmb = [1]
            subspace_lmb = [1]
            radial_profile = "lu_h"
        "#,
            )
            .unwrap();
            // Upstream map/evaluator construction needs a larger stack in debug builds.
            std::thread::scope(|scope| {
                std::thread::Builder::new()
                    .stack_size(64 * 1024 * 1024)
                    .spawn_scoped(scope, || {
                        state.generate_integrands(&global, (&runtime).into())
                    })
                    .unwrap()
                    .join()
                    .unwrap()
                    .unwrap();
            });
            state.save(temp.path(), true, false).unwrap();
            if let Some(output) = std::env::var_os("GAMMABOARD_TEST_STATE_OUTPUT") {
                // Retain an explicitly requested fresh fixture for pipeline benchmarks.
                state
                    .save(std::path::Path::new(&output), false, true)
                    .unwrap();
            }
            temp
        })
        .path()
}

fn params(reference: bool, sampled_channels: bool) -> GammaLoopParams {
    GammaLoopParams {
        state_folder: fixture().to_path_buf(),
        integrand_name: Some("default".into()),
        reference_gaussian: reference
            .then(|| GaussianReferenceFunction::new(1.5, vec![0.2, -0.3, 0.1]).unwrap()),
        preprocessing: GammaLoopPreprocessing {
            commands: if sampled_channels {
                vec!["set process string '[sampling]\ngraphs = \"monte_carlo\"\nsampling_channels = \"monte_carlo\"'".into()]
            } else {
                Vec::new()
            },
            ..Default::default()
        },
        ..Default::default()
    }
}

fn halton(mut index: usize, base: usize) -> f64 {
    let (mut value, mut factor) = (0.0, 1.0);
    while index > 0 {
        factor /= base as f64;
        value += factor * (index % base) as f64;
        index /= base;
    }
    value
}

#[test]
fn reference_maps_preserve_normalization_moments_and_channel_selection() {
    for sampled in [false, true] {
        let config = params(true, sampled);
        let domain = GammaLoopEvaluator::resolve_domain_from_params(config.clone()).unwrap();
        let mut evaluator = GammaLoopEvaluator::from_params(config).unwrap();
        assert_eq!(domain, evaluator.get_domain());
        assert_eq!(
            evaluator.metadata()["sampling_channels"][0]["entries"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let start = std::time::Instant::now();
        let n = 4096;
        let mut total = GammaLoopAccumulatorState::default();
        // Stratify both channels at the same cube points; average their sum once per draw.
        // Each MC branch receives its reciprocal selection probability (2).
        for first in (1..=n).step_by(256) {
            let points = (first..first + 256).flat_map(|i| {
                let continuous = [2, 3, 5].map(|base| halton(i, base)).to_vec();
                (0..if sampled { 2 } else { 1 }).map(move |channel| {
                    Point::new(
                        continuous.clone(),
                        if sampled { vec![0, channel] } else { vec![] },
                        if sampled { 2.0 } else { 1.0 },
                    )
                })
            });
            let result = evaluator
                .eval_batch(
                    &Batch::from_points(points).unwrap(),
                    &AccumulatorConfig::Gammaloop,
                    EvalBatchOptions {
                        require_training_values: false,
                    },
                )
                .unwrap();
            let AccumulatorState::Gammaloop(state) = result.accumulator else {
                panic!("GammaLoop accumulator")
            };
            total.merge_in_place(state).unwrap();
        }
        eprintln!(
            "reference sampled_channels={sampled}: normalization={:.6}, moment={:.6}, samples={}, elapsed={:?}",
            total.real_mean(),
            total.imag_mean(),
            total.sample_count(),
            start.elapsed()
        );
        assert_eq!(total.diagnostics.count_nan_or_unstable, 0);
        // Deterministic quadrature tolerances, not IID confidence intervals.
        assert!(
            (total.real_mean() - 1.0).abs() < 0.04,
            "normalization {}",
            total.real_mean()
        );
        assert!(
            (total.imag_mean() - 1.0).abs() < 0.04,
            "moment {}",
            total.imag_mean()
        );
    }
}

#[test]
fn physical_histograms_and_training_feedback_apply_outer_weights_once() {
    let mut config = params(false, true);
    config.preprocessing.commands.push(
        r#"set process string '
        [quantities.graph_id]
        type = "graph_id"
        [observables.graphs]
        title = "graph"
        type_description = "acceptance"
        quantity = "graph_id"
        phase = "real"
        kind = "discrete"
        domain = { type = "graph_ids" }
        labels = { type = "graph_name" }
    '"#
        .into(),
    );
    let mut evaluator = GammaLoopEvaluator::from_params(config).unwrap();
    let options = EvalBatchOptions {
        require_training_values: true,
    };
    let mut singles = GammaLoopAccumulatorState::default();
    let mut expected = Vec::new();
    let points: Vec<_> = [0.3, 0.5, 0.7]
        .into_iter()
        .enumerate()
        .map(|(i, x)| Point::new(vec![x, 0.41, 0.61], vec![0, (i % 2) as i64], (i + 2) as f64))
        .collect();
    for point in &points {
        let unit = Point::new(point.continuous.clone(), point.discrete.clone(), 1.0);
        let unit = evaluator
            .eval_batch(
                &Batch::new(vec![unit]).unwrap(),
                &AccumulatorConfig::Gammaloop,
                options,
            )
            .unwrap();
        let AccumulatorState::Gammaloop(unit_state) = unit.accumulator else {
            panic!()
        };
        assert!(unit_state.real_mean().is_finite() && unit_state.real_mean() != 0.0);
        expected.push(unit_state.real_mean() * point.total_weight());
        let weighted = evaluator
            .eval_batch(
                &Batch::new(vec![point.clone()]).unwrap(),
                &AccumulatorConfig::Gammaloop,
                options,
            )
            .unwrap();
        let AccumulatorState::Gammaloop(weighted_state) = weighted.accumulator else {
            panic!()
        };
        let unit_hist = &unit_state.bundle.histograms["graphs"];
        let weighted_hist = &weighted_state.bundle.histograms["graphs"];
        for (a, b) in unit_hist.bins.iter().zip(&weighted_hist.bins) {
            let expected_sum = a.sum_weights * point.total_weight();
            assert!((b.sum_weights - expected_sum).abs() < expected_sum.abs() * 1e-10);
            let expected_squared = a.sum_weights_squared * point.total_weight().powi(2);
            assert!(
                (b.sum_weights_squared - expected_squared).abs() < expected_squared.abs() * 1e-10
            );
        }
        singles.merge_in_place(weighted_state).unwrap();
    }
    let batch = Batch::new(points).unwrap();
    for mode in [
        AccumulatorConfig::Gammaloop,
        AccumulatorConfig::scalar(),
        AccumulatorConfig::Empty,
    ] {
        let result = evaluator.eval_batch(&batch, &mode, options).unwrap();
        for (actual, expected) in result.values.as_ref().unwrap().iter().zip(&expected) {
            assert!(
                (actual - expected).abs() < expected.abs() * 1e-10,
                "{actual} != {expected}"
            );
        }
        if let AccumulatorState::Gammaloop(state) = result.accumulator {
            assert_eq!(state.bundle.histograms["graphs"].sample_count, 3);
            assert_eq!(
                serde_json::to_value(&state.bundle).unwrap(),
                serde_json::to_value(&singles.bundle).unwrap()
            );
        }
    }
    // A failed batch must not contaminate the next native observable batch.
    assert!(
        evaluator
            .evaluate(&Batch::new(vec![Point::new(vec![], vec![0, 0], 1.0)]).unwrap())
            .is_err()
    );
    let recovered = evaluator
        .eval_batch(&batch, &AccumulatorConfig::Gammaloop, options)
        .unwrap();
    let AccumulatorState::Gammaloop(recovered) = recovered.accumulator else {
        panic!()
    };
    assert_eq!(
        serde_json::to_value(recovered.bundle).unwrap(),
        serde_json::to_value(singles.bundle).unwrap()
    );
}

#[test]
fn reference_rejects_physics_observables_and_momentum_input() {
    let mut config = params(true, false);
    config.momentum_space = true;
    assert!(
        GammaLoopEvaluator::from_params(config)
            .err()
            .unwrap()
            .to_string()
            .contains("requires x-space")
    );
    let invalid: Result<GammaLoopParams, _> =
        serde_json::from_value(json!({"reference_gaussian": {"width": 0.0}}));
    assert!(invalid.is_err());
    let mut config = params(true, false);
    config.preprocessing.commands.push(
        r#"set process string '
        [quantities.graph_id]
        type = "graph_id"
        [observables.graphs]
        title = "graph"
        type_description = "acceptance"
        quantity = "graph_id"
        phase = "real"
        kind = "discrete"
        domain = { type = "graph_ids" }
        labels = { type = "graph_name" }
    '"#
        .into(),
    );
    assert!(
        GammaLoopEvaluator::from_params(config)
            .err()
            .unwrap()
            .to_string()
            .contains("disable physical selectors and observables")
    );
}

#[test]
fn quad_override_uses_native_stability_levels_and_preserves_the_reference_value() {
    let config = params(true, true);
    let mut double = GammaLoopEvaluator::from_params(config.clone()).unwrap();
    let mut quad = GammaLoopEvaluator::from_params(GammaLoopParams {
        use_f128: true,
        ..config
    })
    .unwrap();
    assert!(
        quad.integrand
            .get_settings()
            .stability
            .levels
            .iter()
            .all(|level| level.precision != Precision::Double)
    );
    let batch = Batch::new(vec![Point::new(vec![0.5, 0.4, 0.6], vec![0, 0], 2.0)]).unwrap();
    let expected = double.evaluate(&batch).unwrap();
    let actual = quad.evaluate(&batch).unwrap();
    let expected = GammaLoopEvaluator::project_result_value(&expected[0]);
    let actual = GammaLoopEvaluator::project_result_value(&actual[0]);
    assert!(expected.re > 0.0 && expected.im > 0.0);
    assert!((actual - expected).norm() < expected.norm() * 1e-8);
}
