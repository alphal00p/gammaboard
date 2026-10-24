use crate::core::{StratificationConfig, StratificationPlan};
use crate::evaluation::EvalBatchOptions;
use crate::evaluation::stratification::integrand_batch;
use crate::sampling::{DiscreteSubspace, Generation, StageHandoff};
use crate::utils::domain::{DiscreteAxisRole, DomainBranch};
use crate::{AccumulatorConfig, Domain, EvaluatorConfig, SamplingConfig};
use serde_json::json;

fn ragged_prefix() -> Domain {
    let node = |branches| Domain::Discrete {
        axis_label: None,
        axis_role: DiscreteAxisRole::Stratum,
        branches,
    };
    node(vec![
        DomainBranch::new(0, Domain::strata(vec![])),
        DomainBranch::new(
            1,
            node(
                [0, 1, 3]
                    .into_iter()
                    .map(|i| DomainBranch::new(i, Domain::strata(vec![])))
                    .collect(),
            ),
        ),
    ])
}

fn partition() -> StratificationConfig {
    StratificationConfig::Voronoi {
        domain: ragged_prefix(),
        centers: [
            (vec![0], vec![0.125]),
            (vec![1, 0], vec![0.375]),
            (vec![1, 1], vec![0.625]),
            (vec![1, 3], vec![0.875]),
        ]
        .into(),
        scales: None,
    }
}

#[test]
fn ragged_strata_are_left_bound_complete_and_prefix_free() {
    let partition = partition();
    partition.validate().unwrap();
    assert_eq!(
        partition.domain().stratum_prefixes().unwrap(),
        vec![vec![0], vec![1, 0], vec![1, 1], vec![1, 3]]
    );
    let augmented = partition
        .sampling_domain(&Domain::rectangular_with_cardinalities(2, [3]))
        .unwrap();
    assert_eq!(
        augmented.stratum_prefixes().unwrap(),
        partition.domain().stratum_prefixes().unwrap()
    );
    for bad_prefix in [vec![1], vec![1, 2], vec![0, 0]] {
        let mut bad = partition.clone();
        let StratificationConfig::Voronoi { centers, .. } = &mut bad;
        centers.remove(&vec![1, 3]);
        centers.insert(bad_prefix, vec![0.875]);
        assert!(bad.validate().is_err());
    }
    let invalid = Domain::discrete(None, [DomainBranch::new(0, Domain::strata(vec![2]))]);
    assert!(invalid.validate().unwrap_err().contains("left-bound"));
    let invalid = Domain::Rectangular {
        strata_dims: 2,
        discrete_cardinalities: vec![2],
        continuous_dims: 1,
    };
    assert!(invalid.validate().is_err());
    let serialized = toml::to_string(&partition).unwrap();
    assert_eq!(
        toml::from_str::<StratificationConfig>(&serialized).unwrap(),
        partition
    );
}

#[test]
fn generic_samplers_preserve_ragged_labels_masses_feedback_and_frozen_replay() {
    for kind in ["naive_monte_carlo", "havana_training"] {
        let partition = partition();
        let mut config: SamplingConfig = serde_json::from_value(if kind == "naive_monte_carlo" {
            json!({"kind":kind,"seed":17,"training_window_samples":1024})
        } else {
            json!({"kind":kind,"seed":17})
        })
        .unwrap();
        config.stratification = Some(partition.clone());
        let domain = Domain::continuous(1);
        let mut sampler = config
            .create(domain.clone(), Some(1024), json!({}))
            .unwrap();
        let queries = partition
            .prefixes()
            .map(|p| DiscreteSubspace {
                fixed_dims: p.iter().copied().enumerate().collect(),
            })
            .collect::<Vec<_>>();
        let probabilities = sampler
            .discrete_mass_batch(&queries)
            .unwrap()
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .unwrap();
        for (p, expected) in probabilities.iter().zip([0.5, 1. / 6., 1. / 6., 1. / 6.]) {
            assert!((p - expected).abs() < 1e-12);
        }
        assert_eq!(
            sampler
                .discrete_mass_batch(&[DiscreteSubspace {
                    fixed_dims: [(0, 1), (1, 2)].into()
                }])
                .unwrap(),
            vec![Some(0.)]
        );
        let Generation::Batch { batch, .. } = sampler.generate(256).unwrap() else {
            panic!("draw")
        };
        let mut materializer = config.build_materializer(&domain, None).unwrap();
        let batch = materializer.materialize_batch(&batch).unwrap();
        let pdf_points = batch
            .points()
            .iter()
            .map(|p| (p.discrete.clone(), p.continuous.clone()))
            .collect::<Vec<_>>();
        for (p, density) in batch
            .points()
            .iter()
            .zip(sampler.pdf_batch(&pdf_points).unwrap())
        {
            let label = partition
                .prefixes()
                .position(|prefix| p.discrete.starts_with(prefix))
                .unwrap();
            assert!((p.total_weight() * probabilities[label] - 1.).abs() < 1e-12);
            if let Some(density) = density {
                assert!((p.total_weight() * density - 1.).abs() < 1e-12);
            } else {
                assert_eq!(kind, "naive_monte_carlo");
            }
        }
        let plan = StratificationPlan {
            proposal_id: "ragged".into(),
            partition: partition.clone(),
            probabilities: None,
            components: vec!["value".into()],
        };
        let (batch, labels) = integrand_batch(batch, &plan).unwrap();
        assert!(batch.points().iter().all(|p| p.discrete.is_empty()));
        let evaluator_config: EvaluatorConfig =
            serde_json::from_value(json!({"kind":"unit","continuous_dims":1,"value_coordinate":0}))
                .unwrap();
        let mut evaluator = evaluator_config.build().unwrap();
        let result = evaluator
            .eval_batch(
                &batch,
                &AccumulatorConfig::scalar(),
                EvalBatchOptions {
                    require_training_values: true,
                    stratification: Some((&plan, &labels)),
                },
            )
            .unwrap();
        let feedback = result.feedback.unwrap();
        let strata = feedback.strata.as_ref().unwrap();
        assert_eq!(
            strata.prefixes,
            partition.domain().stratum_prefixes().unwrap()
        );
        assert!(
            strata
                .requested
                .iter()
                .zip(&strata.actual)
                .any(|(a, b)| a != b)
        );
        for ((p, raw), masked) in batch
            .points()
            .iter()
            .zip(&feedback.unmasked_values)
            .zip(feedback.values().iter())
        {
            assert!((raw - p.continuous[0] * p.total_weight()).abs() < 1e-12);
            assert!(*masked == 0. || masked == raw);
        }
        sampler.feedback(&feedback).unwrap();
        let saved = config.snapshot(&mut *sampler).unwrap();
        let mut restored = saved.clone().restore(&domain, json!({})).unwrap();
        assert_eq!(
            sampler.generate(16).unwrap(),
            restored.generate(16).unwrap()
        );
        if kind == "havana_training" {
            let inference: SamplingConfig =
                serde_json::from_value(json!({"kind":"havana_inference"})).unwrap();
            let inference = inference.inherit_stratification(&saved).unwrap();
            let handoff = StageHandoff {
                sampler_snapshot: Some(&saved),
                observable_state: None,
            };
            let mut frozen = inference.from_stage(&domain, handoff).unwrap();
            let mut materializer = inference
                .build_materializer(&domain, Some(handoff))
                .unwrap();
            let batch = materializer
                .materialize_batch(&frozen.generate(256).unwrap().into_batch().unwrap())
                .unwrap();
            let (_, labels) = integrand_batch(batch, &plan).unwrap();
            assert!(
                labels.contains(&3),
                "sparse label (1,3) survives frozen replay"
            );
        }
    }
}
