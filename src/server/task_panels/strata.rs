use super::{
    TaskPanelCurrentSourcePolicy, TaskPanelProjector, panel_projector_with_source, sample,
};
use crate::core::{AccumulatorConfig, BasicEstimate, EstimateKind};
use crate::evaluation::AccumulatorState;
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelState, PanelWidth, StrataFrame, StratumRow, sized_panel_spec,
};

pub(super) fn projector(config: AccumulatorConfig) -> TaskPanelProjector {
    let history = config.clone();
    panel_projector_with_source(
        sized_panel_spec(
            "strata",
            "Strata",
            PanelKind::Strata,
            PanelHistoryMode::Append,
            PanelWidth::Full,
        ),
        TaskPanelCurrentSourcePolicy::PersistedFirst,
        move |ctx| Ok(sample::sample_accumulator(ctx, &config)?.and_then(panel)),
        move |ctx| {
            Ok(sample::decode_history_observable(ctx, &history)?
                .and_then(panel)
                .map(|mut p| {
                    if let PanelState::Strata { frames, .. } = &mut p {
                        frames[0].wall_time_ms =
                            ctx.snapshot.created_at.map(|t| t.timestamp_millis());
                        frames[0].sampler_uptime_ms = ctx.snapshot.sampler_uptime_ms;
                    }
                    p
                }))
        },
    )
}

fn panel(accumulator: AccumulatorState) -> Option<PanelState> {
    let (components, estimator, rows, total, basic) = match &accumulator {
        AccumulatorState::Stratified(state) => {
            // Moments may also contain an auxiliary norm training projection.
            let width = state.strata[0].mean.len();
            let components = state.plan.components.len();
            let covariance = state.covariance();
            let means = state.mean();
            let total = (0..components)
                .map(|j| {
                    means.as_ref().map(|m| BasicEstimate {
                        mean: m[j],
                        error: covariance
                            .as_ref()
                            .map(|c| c[j * width + j].max(0.0).sqrt()),
                    })
                })
                .collect();
            let rows = state
                .strata
                .iter()
                .enumerate()
                .map(|(k, s)| StratumRow {
                    prefix: state.plan.partition.prefix(k).to_vec(),
                    attempts: s.attempts,
                    matches: s.matches,
                    probability: state.plan.probabilities.as_ref().map(|p| p[k]),
                    estimates: (0..components)
                        .map(|j| {
                            (s.attempts > 0).then(|| BasicEstimate {
                                mean: s.mean[j],
                                error: s.standard_error(j),
                            })
                        })
                        .collect(),
                })
                .collect();
            (
                state.plan.components.clone(),
                EstimateKind::Stratified,
                rows,
                total,
                state.basic_estimates().map(|mut estimates| {
                    estimates.truncate(components);
                    estimates
                }),
            )
        }
        _ => {
            let vector = accumulator.pooled_vector()?;
            let partition = vector.partition.as_ref()?;
            let rows = partition
                .partition
                .prefixes()
                .enumerate()
                .map(|(k, p)| StratumRow {
                    prefix: p.clone(),
                    attempts: partition.attempts[k],
                    matches: partition.matches[k],
                    probability: None,
                    estimates: vec![],
                })
                .collect();
            let total = vector
                .components
                .iter()
                .map(|c| {
                    (c.state.count > 0).then(|| BasicEstimate {
                        mean: c.state.mean(),
                        error: c.state.stderr(),
                    })
                })
                .collect();
            (
                vector.components.iter().map(|c| c.name.clone()).collect(),
                EstimateKind::Pooled,
                rows,
                total,
                None,
            )
        }
    };
    Some(PanelState::Strata {
        panel_id: "strata".into(),
        components,
        estimator,
        frames: vec![StrataFrame {
            samples: accumulator.sample_count().max(0) as u64,
            wall_time_ms: None,
            sampler_uptime_ms: None,
            rows,
            total,
            basic,
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{StratificationConfig, StratificationPlan};
    use crate::evaluation::accumulator::{PartitionDiagnostics, StratifiedAccumulatorState};

    fn partition() -> StratificationConfig {
        StratificationConfig::Voronoi {
            domain: crate::Domain::strata(vec![2]),
            centers: [(vec![0], vec![-1.]), (vec![1], vec![1.])].into(),
            scales: None,
        }
    }
    #[test]
    fn plots_keep_conditional_estimates_separate_from_pooled_training_counts() {
        let config = AccumulatorConfig::scalar();
        let mut training = AccumulatorState::from_config(&config);
        let AccumulatorState::Vector(vector) = &mut training else {
            panic!()
        };
        vector.partition = Some(PartitionDiagnostics {
            partition: partition(),
            attempts: vec![0, 0],
            matches: vec![0, 0],
            feedback_samples: 0,
        });
        let PanelState::Strata {
            frames, estimator, ..
        } = panel(training).unwrap()
        else {
            panic!()
        };
        assert_eq!(estimator, EstimateKind::Pooled);
        assert!(
            frames[0]
                .rows
                .iter()
                .all(|r| r.probability.is_none() && r.estimates.is_empty())
        );
        let plan = StratificationPlan {
            proposal_id: "plot".into(),
            partition: partition(),
            probabilities: Some(vec![0.2, 0.8]),
            components: vec!["value".into()],
        };
        let mut state = StratifiedAccumulatorState::new(plan, config).unwrap();
        for _ in 0..32 {
            state.push(0, &[-2.], 5., true).unwrap();
            state.push(1, &[3.], 1.25, true).unwrap();
        }
        let PanelState::Strata { frames, .. } =
            panel(AccumulatorState::Stratified(Box::new(state))).unwrap()
        else {
            panic!()
        };
        assert_eq!(frames[0].total[0].as_ref().unwrap().mean, 1.);
        assert_eq!(frames[0].rows[0].estimates[0].as_ref().unwrap().mean, -2.);
        assert_eq!(frames[0].rows[1].estimates[0].as_ref().unwrap().mean, 3.);
        assert_ne!(frames[0].basic.as_ref().unwrap()[0].mean, 1.);
    }

    #[test]
    fn vector_norm_total_errors_match_primary_measurements() {
        use crate::core::{StratificationConfig, StratificationPlan, TrainingProjection};
        use crate::evaluation::accumulator::StratifiedAccumulatorState;
        let config =
            AccumulatorConfig::vector(vec!["x".into(), "y".into()], TrainingProjection::Norm);
        let plan = StratificationPlan {
            proposal_id: "vector-plot".into(),
            components: vec!["x".into(), "y".into()],
            probabilities: Some(vec![0.4, 0.6]),
            partition: StratificationConfig::Voronoi {
                domain: crate::Domain::strata(vec![2]),
                centers: [(vec![0], vec![-1., 1.]), (vec![1], vec![1., -1.])].into(),
                scales: None,
            },
        };
        let mut state = StratifiedAccumulatorState::new(plan, config).unwrap();
        for i in 0..64 {
            let v = 1. + i as f64 / 100.;
            state.push(i % 2, &[v, -2. * v], 1., true).unwrap();
        }
        let accumulator = AccumulatorState::Stratified(Box::new(state));
        let PanelState::Strata { frames, .. } = panel(accumulator.clone()).unwrap() else {
            panic!()
        };
        for (j, component) in ["x", "y"].into_iter().enumerate() {
            let selector =
                serde_json::from_value(serde_json::json!({"name":"mean","component":component}))
                    .unwrap();
            let metric = crate::evaluation::extract_accumulator_metric(&accumulator, &selector)
                .unwrap()
                .unwrap();
            let plotted = frames[0].total[j].as_ref().unwrap();
            assert_eq!(plotted.mean, metric.value);
            assert_eq!(plotted.error, metric.uncertainty);
            assert!(plotted.error.unwrap() > 0.);
            assert_eq!(frames[0].rows[0].estimates.len(), 2);
        }
    }
}
