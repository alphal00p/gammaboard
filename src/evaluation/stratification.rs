//! Value-level gating shared by native and process evaluators.
use super::accumulator::StratifiedAccumulatorState;
use super::{AccumulatorState, EvalBatchOptions};
use crate::core::{AccumulatorConfig, StratificationPlan, TrainingProjection};
use crate::{Batch, BatchResult, EngineError, Point};

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
    stratification: Option<(&StratificationPlan, &[usize])>,
) -> Result<BatchResult, EngineError> {
    let width = components.len();
    if width == 0 || values.len() != batch.len() * width || values.iter().any(|v| !v.is_finite()) {
        return Err(EngineError::eval("invalid per-sample evaluator values"));
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
            if labels[i] >= plan.partition.centers().len() {
                return Err(EngineError::eval("invalid stratum label"));
            }
            matched = plan.partition.classify(raw)? == labels[i];
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
                0.
            }
            AccumulatorState::Empty(_) => gated[0],
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
    if matches!(state, AccumulatorState::FullVector(_)) {
        feedback = None;
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
            },
            Some((&plan, &labels)),
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
            },
            Some((&plan, &labels)),
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
