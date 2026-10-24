//! Value-level gating shared by native and process evaluators.
use super::accumulator::StratifiedAccumulatorState;
use super::{AccumulatorState, EvalBatchOptions, IngestScalar};
use crate::core::{AccumulatorConfig, StratificationPlan, TrainingProjection};
use crate::{Batch, BatchResult, EngineError, Point};

/// Initialize or restore the one immutable plan used by both worker roles.
/// Restoring never queries the sampler's current probability table.
pub(crate) fn prepare_estimator(
    task: &crate::core::RunTaskSpec,
    sampling: &crate::core::SamplingConfig,
    sampler: &mut dyn crate::sampling::Sampler,
    state: &mut AccumulatorState,
    restoring: bool,
) -> Result<Option<StratificationPlan>, EngineError> {
    use crate::core::{Estimator, RunTaskSpec, StratificationConfig, tasks::StageSourceSpec};
    let stratified = matches!(
        task,
        RunTaskSpec::Sample {
            estimator: Estimator::Stratified,
            ..
        }
    );
    let Some(partition) = &sampling.stratification else {
        if stratified || matches!(state, AccumulatorState::Stratified(_)) {
            return Err(EngineError::invalid_input(
                "stratified estimation requires a matching sampling partition",
            ));
        }
        return Ok(None);
    };
    let config = state.config();
    partition.validate_accumulator(&config)?;
    if let AccumulatorState::Stratified(saved) = state {
        if !stratified || !restoring || &saved.plan.partition != partition {
            return Err(EngineError::invalid_input(
                "incompatible stratification accumulator handoff",
            ));
        }
        saved.validate()?;
        return Ok(Some(saved.plan.clone()));
    }
    if stratified
        && (restoring
            || state.sample_count() != 0
            || !matches!(
                task,
                RunTaskSpec::Sample {
                    accumulator: Some(StageSourceSpec::Config { .. }),
                    ..
                }
            ))
    {
        return Err(EngineError::invalid_input(
            "stratified inference requires an explicit fresh accumulator",
        ));
    }
    let probabilities = if stratified {
        let queries = partition
            .prefixes()
            .map(|prefix| crate::sampling::DiscreteSubspace {
                fixed_dims: prefix.iter().copied().enumerate().collect(),
            })
            .collect::<Vec<_>>();
        Some(
            sampler
                .discrete_mass_batch(&queries)?
                .into_iter()
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| {
                    EngineError::invalid_input(
                        "sampler must provide exact discrete probabilities for stratification",
                    )
                })?,
        )
    } else {
        None
    };
    let plan = StratificationPlan {
        proposal_id: uuid::Uuid::new_v4().to_string(),
        partition: partition.clone(),
        probabilities,
        components: StratificationConfig::components(&config)?,
    };
    plan.validate()?;
    if stratified {
        *state = AccumulatorState::Stratified(Box::new(StratifiedAccumulatorState::new(
            plan.clone(),
            config,
        )?));
    }
    Ok(Some(plan))
}

/// Native and process adapters share the same hybrid finalization. Histogram
/// contributions are already masked and fully weighted; only integral moments
/// receive the additional p_k correction.
pub fn finish_gammaloop(
    batch: &Batch,
    values: Option<&[f64]>,
    mut state: super::GammaLoopAccumulatorState,
    training_values: Option<Vec<f64>>,
    options: EvalBatchOptions<'_>,
) -> Result<BatchResult, EngineError> {
    state.estimate.partition = options.partition_diagnostics(values.unwrap_or(&[]), 2)?;
    state.validate_values()?;
    let components = ["real".into(), "imag".into()];
    options.validate(batch.len(), &components)?;
    let accumulator = if let Some((plan, _)) = options.stratification {
        state.validate_stratified_samples(batch.len() as i64)?;
        let values = values.ok_or_else(|| {
            EngineError::eval("GammaLoop stratification requires per-sample real/imag values")
        })?;
        if values.len() != batch.len() * 2 || values.iter().any(|v| !v.is_finite()) {
            return Err(EngineError::eval("invalid GammaLoop stratification values"));
        }
        // Observable producers return aggregate state, but the partition context
        // and raw values let the host retain complete, reproducible extrema too.
        annotate_observable_extrema(&mut state, batch, values, options)?;
        if plan.probabilities.is_some() {
            let result = accumulate_values(
                batch,
                &components,
                values,
                &AccumulatorConfig::Gammaloop,
                EvalBatchOptions {
                    require_training_values: false,
                    ..options
                },
            )?;
            let AccumulatorState::Stratified(mut estimate) = result.accumulator else {
                unreachable!()
            };
            estimate.pooled = Some(Box::new(state));
            estimate.validate()?;
            AccumulatorState::Stratified(estimate)
        } else {
            AccumulatorState::Gammaloop(state)
        }
    } else {
        AccumulatorState::Gammaloop(state)
    };
    Ok(BatchResult::new(
        options.feedback(training_values, values.unwrap_or(&[]), 2)?,
        accumulator,
    ))
}

fn annotate_observable_extrema(
    state: &mut super::GammaLoopAccumulatorState,
    batch: &Batch,
    values: &[f64],
    options: EvalBatchOptions<'_>,
) -> Result<(), EngineError> {
    fn update(
        state: &mut super::ScalarAccumulatorState,
        batch: &Batch,
        values: &[f64],
        options: EvalBatchOptions<'_>,
        project: impl Fn(&[f64]) -> f64,
    ) -> Result<(), EngineError> {
        let previous = [
            state.max_weighted_positive_sample.take(),
            state.max_weighted_negative_sample.take(),
        ];
        state.max_weighted_positive = 0.0;
        state.max_weighted_negative = 0.0;
        for (i, (point, raw)) in batch
            .points()
            .iter()
            .zip(values.as_chunks::<2>().0.iter())
            .enumerate()
        {
            let assignment = options.stratum_assignment(i, raw)?;
            let matched = assignment.is_none_or(|(a, b)| a == b);
            let value = if matched {
                project(raw) * point.total_weight()
            } else {
                0.0
            };
            if !value.is_finite() {
                return Err(EngineError::eval(
                    "non-finite observable diagnostic contribution",
                ));
            }
            let evaluation = previous
                .iter()
                .flatten()
                .find(|saved| saved.point == *point)
                .and_then(|saved| saved.evaluation.as_ref());
            state.update_extrema(value, point, evaluation, assignment);
        }
        Ok(())
    }
    for (j, component) in state.estimate.components.iter_mut().enumerate() {
        update(&mut component.state, batch, values, options, |raw| raw[j])?;
    }
    if state.estimate.projection_spec() == TrainingProjection::Norm {
        update(
            &mut state.estimate.projection_mut().state,
            batch,
            values,
            options,
            |raw| raw[0].hypot(raw[1]),
        )?;
    }
    if let Some(auxiliary) = &mut state.training_norm_sqr {
        update(auxiliary, batch, values, options, |raw| {
            raw[0].hypot(raw[1]).powi(2)
        })?;
    }
    Ok(())
}

/// Remove the complete runtime-owned stratum prefix before integrand evaluation.
pub fn integrand_batch(
    batch: Batch,
    plan: &StratificationPlan,
) -> Result<(Batch, Vec<usize>), EngineError> {
    plan.validate()?;
    let mut labels = Vec::with_capacity(batch.len());
    let points = batch
        .into_points()
        .into_iter()
        .map(|mut point| {
            let (label, prefix) = plan
                .partition
                .prefixes()
                .enumerate()
                .find(|(_, p)| point.discrete.starts_with(p))
                .ok_or_else(|| EngineError::eval("missing or invalid stratum prefix"))?;
            let width = prefix.len();
            point.discrete.drain(..width);
            labels.push(label);
            Ok(point)
        })
        .collect::<Result<Vec<Point>, EngineError>>()?;
    Ok((Batch::new(points), labels))
}

pub fn accumulate_values(
    batch: &Batch,
    components: &[String],
    values: &[f64],
    config: &AccumulatorConfig,
    options: EvalBatchOptions,
) -> Result<BatchResult, EngineError> {
    let stratification = options.stratification;
    let width = components.len();
    if width == 0 || values.len() != batch.len() * width || values.iter().any(|v| !v.is_finite()) {
        return Err(EngineError::eval("invalid per-sample evaluator values"));
    }
    if options.require_training_values
        && matches!(config, AccumulatorConfig::FullVector { .. })
        && width != 1
    {
        return Err(EngineError::invalid_input(
            "full_vector training feedback requires a scalar output",
        ));
    }
    let mut state = if let Some((plan, labels)) = stratification {
        plan.validate()?;
        plan.partition.validate_accumulator(config)?;
        if labels.len() != batch.len() || components != plan.components {
            return Err(EngineError::eval("stratification batch layout mismatch"));
        }
        if plan.probabilities.is_some() {
            AccumulatorState::Stratified(Box::new(StratifiedAccumulatorState::new(
                plan.clone(),
                config.clone(),
            )?))
        } else {
            AccumulatorState::from_config(config)
        }
    } else {
        AccumulatorState::from_config(config)
    };
    let mut diagnostics = if matches!(state, AccumulatorState::Vector(_)) {
        stratification.map(|(plan, _)| super::accumulator::PartitionDiagnostics {
            partition: plan.partition.clone(),
            attempts: vec![0; plan.partition.centers().len()],
            matches: vec![0; plan.partition.centers().len()],
            feedback_samples: if options.require_training_values {
                batch.len() as u64
            } else {
                0
            },
        })
    } else {
        None
    };
    let mut feedback = options
        .require_training_values
        .then(|| Vec::with_capacity(batch.len()));
    for (i, (point, raw)) in batch
        .points()
        .iter()
        .zip(values.chunks_exact(width))
        .enumerate()
    {
        let mut gated = raw.to_vec();
        let assignment = if let Some((plan, labels)) = stratification {
            let actual = plan.partition.classify(raw)?;
            if let Some(diagnostics) = &mut diagnostics {
                diagnostics.attempts[labels[i]] += 1;
                diagnostics.matches[labels[i]] += u64::from(labels[i] == actual);
            }
            Some((
                plan.partition.prefix(labels[i]),
                plan.partition.prefix(actual),
            ))
        } else {
            None
        };
        let matched = assignment.is_none_or(|(requested, actual)| requested == actual);
        if !matched {
            gated.fill(0.0);
        }
        match &mut state {
            AccumulatorState::Vector(vector) => {
                vector.ingest_vector_with_details(&gated, point, None, assignment)?;
            }
            AccumulatorState::Stratified(accumulator) => {
                let label = stratification.expect("stratified context").1[i];
                accumulator.push_with_diagnostics(
                    label,
                    &gated,
                    point.total_weight(),
                    matched,
                    assignment.map(|assignment| (point, assignment)),
                )?;
            }
            AccumulatorState::FullVector(full) => {
                full.push_vector(
                    &gated
                        .iter()
                        .map(|x| x * point.total_weight())
                        .collect::<Vec<_>>(),
                )?;
            }
            AccumulatorState::Empty(empty) => {
                empty.ingest_scalar(gated[0], point)?;
            }
            _ => {
                return Err(EngineError::invalid_input(
                    "aggregate observable output cannot be gated after accumulation",
                ));
            }
        };
        if let Some(feedback) = &mut feedback {
            let projected = match config {
                AccumulatorConfig::Vector {
                    training_projection: TrainingProjection::Component { name },
                    ..
                } => {
                    raw[components
                        .iter()
                        .position(|n| n == name)
                        .expect("validated projection")]
                }
                AccumulatorConfig::Vector {
                    training_projection: TrainingProjection::Norm,
                    ..
                }
                | AccumulatorConfig::Gammaloop => raw.iter().fold(0.0_f64, |n, x| n.hypot(*x)),
                _ => raw[0],
            };
            feedback.push(projected * point.total_weight());
        }
    }
    if let AccumulatorState::Vector(vector) = &mut state {
        vector.partition = diagnostics;
    }
    Ok(BatchResult::new(
        options.feedback(feedback, values, width)?,
        state,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EvaluatorConfig,
        core::{StratificationConfig, TrainingProjection},
    };
    #[test]
    fn vector_symbolica_gating_preserves_misses_and_full_weight_feedback() {
        let config:EvaluatorConfig=serde_json::from_value(serde_json::json!({"kind":"symbolica","args":["t"],"source":{"kind":"expressions","components":["x","y","z"],"exprs":["t","2*t","-t"]}})).unwrap();
        let mut evaluator = config.build().unwrap();
        let batch = Batch::from_iter([
            Point::new(vec![1.], vec![0], 4.),
            Point::new(vec![-1.], vec![0], 4.),
            Point::new(vec![-1.], vec![1], 4. / 3.),
        ]);
        let plan = StratificationPlan {
            proposal_id: "vector-fixture".into(),
            partition: StratificationConfig::Voronoi {
                domain: crate::Domain::strata(vec![2]),
                centers: vec![vec![1., 2., -1.], vec![-1., -2., 1.]]
                    .into_iter()
                    .enumerate()
                    .map(|(k, c)| (vec![k as i64], c))
                    .collect(),
                scales: None,
            },
            probabilities: None,
            components: vec!["x".into(), "y".into(), "z".into()],
        };
        let (integrand, labels) = integrand_batch(batch, &plan).unwrap();
        assert!(integrand.points().iter().all(|p| p.discrete.is_empty()));
        let values = evaluator.eval_values(&integrand, &plan.components).unwrap();
        let accumulator =
            AccumulatorConfig::vector(plan.components.clone(), TrainingProjection::Norm);
        let train = accumulate_values(
            &integrand,
            &plan.components,
            &values,
            &accumulator,
            EvalBatchOptions {
                require_training_values: true,
                stratification: Some((&plan, &labels)),
            },
        )
        .unwrap();
        assert_eq!(train.accumulator.sample_count(), 3);
        let AccumulatorState::Vector(training) = &train.accumulator else {
            panic!("pooled training")
        };
        let diagnostics = training.partition.as_ref().unwrap();
        assert_eq!(diagnostics.attempts, vec![2, 1]);
        assert_eq!(diagnostics.matches, vec![1, 1]);
        assert_eq!(diagnostics.feedback_samples, 3);
        let source = training.components[0]
            .state
            .max_weighted_positive_sample
            .as_ref()
            .unwrap();
        assert_eq!(source.stratum.as_ref().unwrap().requested, vec![0]);
        assert_eq!(source.stratum.as_ref().unwrap().actual, vec![0]);
        let restored: AccumulatorState =
            serde_json::from_value(serde_json::to_value(&train.accumulator).unwrap()).unwrap();
        let mut merged = train.accumulator.clone();
        merged.merge(restored).unwrap();
        let AccumulatorState::Vector(merged) = merged else {
            unreachable!()
        };
        assert_eq!(merged.partition.as_ref().unwrap().attempts, vec![4, 2]);
        let feedback = train.feedback.unwrap().values().into_owned();
        assert_eq!(feedback[1], 0.);
        assert!((feedback[0] - 4. * 6f64.sqrt()).abs() < 1e-12);
        let plan = StratificationPlan {
            probabilities: Some(vec![0.25, 0.75]),
            ..plan
        };
        let result = accumulate_values(
            &integrand,
            &plan.components,
            &values,
            &accumulator,
            EvalBatchOptions {
                require_training_values: false,
                stratification: Some((&plan, &labels)),
            },
        )
        .unwrap();
        let AccumulatorState::Stratified(state) = result.accumulator else {
            panic!()
        };
        assert_eq!(state.strata[0].attempts, 2);
        assert_eq!(state.strata[0].matches, 1);
        assert_eq!(&state.strata[0].mean[..3], &[0.5, 1., -0.5]);
        assert_eq!(&state.strata[1].mean[..3], &[-1., -2., 1.]);
        assert!(state.covariance().is_none());
    }
    #[test]
    fn queue_plan_roundtrip_separates_stratum_from_integrand_coordinates() {
        let config = AccumulatorConfig::scalar();
        let plan = StratificationPlan {
            proposal_id: "queue".into(),
            partition: crate::core::StratificationConfig::Voronoi {
                domain: crate::Domain::strata(vec![2]),
                centers: vec![vec![0.], vec![1.]]
                    .into_iter()
                    .enumerate()
                    .map(|(k, c)| (vec![k as i64], c))
                    .collect(),
                scales: None,
            },
            probabilities: Some(vec![0.4, 0.6]),
            components: vec!["value".into()],
        };
        let concrete = Batch::from_iter([
            Point::new(vec![0.2], vec![1, 3], 5.),
            Point::new(vec![0.9], vec![0, 2], 7.),
        ]);
        let mut work = crate::LatentBatch::from_batch(&concrete).for_evaluation(config);
        work.stratification = Some(plan.clone());
        let restored = crate::core::EvaluationBatch::from_bytes(&work.to_bytes().unwrap()).unwrap();
        assert_eq!(restored, work);
        let (integrand, labels) =
            integrand_batch(restored.samples.payload.into_batch().unwrap(), &plan).unwrap();
        assert_eq!(labels, vec![1, 0]);
        assert_eq!(integrand.points()[0].discrete, vec![3]);
        assert_eq!(integrand.points()[1].discrete, vec![2]);
        assert_eq!(integrand.weights(), vec![5., 7.]);
    }
}
