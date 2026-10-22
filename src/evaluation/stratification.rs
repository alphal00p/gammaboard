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
        let queries = (0..partition.centers().len())
            .map(|k| crate::sampling::DiscreteSubspace {
                fixed_dims: std::collections::BTreeMap::from([(0, k as i64)]),
            })
            .collect::<Vec<_>>();
        Some(
            sampler
                .discrete_pdf_batch(&queries)?
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
    state: super::GammaLoopAccumulatorState,
    training_values: Option<Vec<f64>>,
    options: EvalBatchOptions<'_>,
) -> Result<BatchResult, EngineError> {
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
        if let Some(feedback) = &training_values {
            if feedback.len() != batch.len() {
                return Err(EngineError::eval(
                    "GammaLoop training feedback length mismatch",
                ));
            }
            for (i, raw) in values.as_chunks::<2>().0.iter().enumerate() {
                if !options.matches(i, raw)? && feedback[i] != 0.0 {
                    return Err(EngineError::eval(
                        "stratum misses must return zero training feedback",
                    ));
                }
            }
        }
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
    Ok(BatchResult::new(training_values, accumulator))
}

/// Remove only the runtime-owned outer label before physical transformations/evaluation.
pub fn physical_batch(
    batch: Batch,
    plan: &StratificationPlan,
) -> Result<(Batch, Vec<usize>), EngineError> {
    plan.validate()?;
    let mut labels = Vec::with_capacity(batch.len());
    let points = batch
        .into_points()
        .into_iter()
        .map(|mut point| {
            let label = point
                .discrete
                .first()
                .copied()
                .and_then(|k| usize::try_from(k).ok())
                .filter(|k| *k < plan.partition.centers().len())
                .ok_or_else(|| EngineError::eval("missing or invalid stratum coordinate"))?;
            point.discrete.remove(0);
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
        let mut matched = true;
        if let Some((plan, labels)) = stratification {
            matched = plan.matches(labels[i], raw)?;
            if !matched {
                gated.fill(0.);
            }
        }
        let projected = match &mut state {
            AccumulatorState::Vector(vector) => vector.ingest_vector(&gated, point)?,
            AccumulatorState::Stratified(accumulator) => {
                let label = stratification.expect("stratified context").1[i];
                accumulator.push(label, &gated, point.total_weight(), matched)?;
                match config {
                    AccumulatorConfig::Vector {
                        training_projection: TrainingProjection::Norm,
                        ..
                    } => gated.iter().fold(0.0_f64, |n, x| n.hypot(*x)),
                    AccumulatorConfig::Vector {
                        training_projection: TrainingProjection::Component { name },
                        ..
                    } => {
                        gated[components
                            .iter()
                            .position(|n| n == name)
                            .expect("validated projection")]
                    }
                    AccumulatorConfig::Gammaloop => gated.iter().fold(0.0_f64, |n, x| n.hypot(*x)),
                    _ => unreachable!(),
                }
            }
            AccumulatorState::FullVector(full) => {
                full.push_vector(
                    &gated
                        .iter()
                        .map(|x| x * point.total_weight())
                        .collect::<Vec<_>>(),
                )?;
                gated[0]
            }
            AccumulatorState::Empty(empty) => {
                empty.ingest_scalar(gated[0], point)?;
                gated[0]
            }
            _ => {
                return Err(EngineError::invalid_input(
                    "aggregate observable output cannot be gated after accumulation",
                ));
            }
        };
        if let Some(feedback) = &mut feedback {
            feedback.push(projected * point.total_weight());
        }
    }
    Ok(BatchResult::new(feedback, state))
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
                centers: vec![vec![1., 2., -1.], vec![-1., -2., 1.]],
                scales: None,
            },
            probabilities: None,
            components: vec!["x".into(), "y".into(), "z".into()],
        };
        let (physical, labels) = physical_batch(batch, &plan).unwrap();
        assert!(physical.points().iter().all(|p| p.discrete.is_empty()));
        let values = evaluator.eval_values(&physical, &plan.components).unwrap();
        let accumulator =
            AccumulatorConfig::vector(plan.components.clone(), TrainingProjection::Norm);
        let train = accumulate_values(
            &physical,
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
        let feedback = train.training_values.unwrap();
        assert_eq!(feedback[1], 0.);
        assert!((feedback[0] - 4. * 6f64.sqrt()).abs() < 1e-12);
        let plan = StratificationPlan {
            probabilities: Some(vec![0.25, 0.75]),
            ..plan
        };
        let result = accumulate_values(
            &physical,
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
    fn queue_plan_roundtrip_separates_stratum_from_physical_coordinates() {
        let config = AccumulatorConfig::scalar();
        let plan = StratificationPlan {
            proposal_id: "queue".into(),
            partition: crate::core::StratificationConfig::Voronoi {
                centers: vec![vec![0.], vec![1.]],
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
        let (physical, labels) =
            physical_batch(restored.samples.payload.into_batch().unwrap(), &plan).unwrap();
        assert_eq!(labels, vec![1, 0]);
        assert_eq!(physical.points()[0].discrete, vec![3]);
        assert_eq!(physical.points()[1].discrete, vec![2]);
        assert_eq!(physical.weights(), vec![5., 7.]);
    }
}
