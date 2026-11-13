//! Runtime-owned weighting and reduction of independent sample outputs.
use super::{AccumulatorState, Batch, BatchResult, EvalBatchOptions, Evaluator};
use crate::core::{AccumulatorConfig, EngineError};

/// One original draw. Observable snapshots describe this sample alone, before
/// outer weights or masking; correlated event fills are already summed per bin.
#[derive(Debug)]
pub struct SampleOutput {
    pub values: Vec<f64>,
    pub evaluation: Option<super::SampleEvaluation>,
    pub diagnostics: super::GammaLoopDiagnostics,
    #[cfg(feature = "gammaloop")]
    pub observables: Option<gammalooprs::observables::ObservableSnapshotBundle>,
}
impl SampleOutput {
    pub fn values(values: Vec<f64>) -> Self {
        Self {
            values,
            evaluation: None,
            diagnostics: Default::default(),
            #[cfg(feature = "gammaloop")]
            observables: None,
        }
    }
}

pub(super) fn evaluate_batch<T: Evaluator + ?Sized>(
    evaluator: &mut T,
    batch: &Batch,
    config: &AccumulatorConfig,
    options: EvalBatchOptions<'_>,
) -> Result<BatchResult, EngineError> {
    use super::evaluator::gammaloop::TrainingProjection;
    let native = matches!(config, AccumulatorConfig::Gammaloop);
    let components = if native {
        vec!["real".into(), "imag".into()]
    } else {
        config.components().unwrap_or(&["value".into()]).to_vec()
    };
    options.validate(batch.len(), &components)?;
    let conditional = options.stratification.and_then(|(plan, labels)| {
        plan.probabilities
            .as_deref()
            .map(|probabilities| (labels, probabilities))
    });
    let projection = evaluator.training_projection();
    let mut observable = native.then(|| super::GammaLoopAccumulatorState {
        training_projection: Some(projection),
        training_norm_sqr: (projection == TrainingProjection::AbsSq).then(|| {
            Box::new(super::ScalarAccumulatorState::from_config(
                None,
                crate::core::AccumulatorMomentConfig::MaxOrder4,
            ))
        }),
        ..Default::default()
    });
    let mut raw = Vec::with_capacity(batch.len() * components.len());
    let mut feedback =
        (native && options.require_training_values).then(|| Vec::with_capacity(batch.len()));
    let mut training_moments = options.stratification.map(|(plan, _)| {
        vec![super::accumulator::TrainingMoments::default(); plan.partition.centers().len()]
    });
    #[cfg(feature = "gammaloop")]
    let mut histograms = Vec::new();
    let mut count = 0;
    evaluator.eval_samples(batch, &components, &mut |sample| {
        let point = batch
            .points()
            .get(count)
            .ok_or_else(|| EngineError::eval("evaluator emitted more results than inputs"))?;
        if sample.values.len() != components.len() || sample.values.iter().any(|x| !x.is_finite()) {
            return Err(EngineError::eval("invalid per-sample evaluator values"));
        }
        if let Some(state) = &mut observable {
            let assignment = options.stratum_assignment(count, &sample.values)?;
            let matched = assignment.is_none_or(|(a, b)| a == b);
            let gated = if matched {
                sample.values.as_slice()
            } else {
                &[0.; 2]
            };
            state.estimate.ingest_vector_with_details(
                gated,
                point,
                sample.evaluation.as_ref(),
                assignment,
            )?;
            let projected =
                projection.project(num::Complex::new(sample.values[0], sample.values[1]));
            if let Some(moments) = &mut training_moments {
                let label = options.stratification.expect("partitioned diagnostics").1[count];
                moments[label].add(if matched {
                    projected * point.total_weight()
                } else {
                    0.0
                })?;
            }
            if let Some(aux) = &mut state.training_norm_sqr {
                aux.add_sample_without_discrete_projection(
                    if matched { projected } else { 0. },
                    point,
                )?;
            }
            if let Some(values) = &mut feedback {
                values.push(projected * point.total_weight());
            }
            state.diagnostics.merge_in_place(sample.diagnostics);
            #[cfg(feature = "gammaloop")]
            {
                let mut bundle = sample.observables.ok_or_else(|| {
                    EngineError::eval("observable evaluator must emit one bundle per sample")
                })?;
                validate_sample_bundle(&bundle)?;
                if count == 0 {
                    state.bundle = super::GammaLoopAccumulatorState::cleared_bundle(&bundle);
                    if let Some((_, probabilities)) = conditional {
                        histograms = vec![state.bundle.clone(); probabilities.len()];
                    }
                }
                if !matched {
                    bundle = super::GammaLoopAccumulatorState::cleared_bundle(&bundle);
                    for histogram in bundle.histograms.values_mut() {
                        histogram.sample_count = 1;
                    }
                }
                rescale_bundle(&mut bundle, point.total_weight());
                state
                    .bundle
                    .merge_in_place(&bundle)
                    .map_err(|e| EngineError::eval(e.to_string()))?;
                if let Some((labels, probabilities)) = conditional {
                    let k = labels[count];
                    rescale_bundle(&mut bundle, probabilities[k]);
                    histograms[k]
                        .merge_in_place(&bundle)
                        .map_err(|e| EngineError::eval(e.to_string()))?;
                }
            }
            #[cfg(not(feature = "gammaloop"))]
            return Err(EngineError::eval(
                "observable reduction requires the gammaloop feature",
            ));
        }
        raw.extend(sample.values);
        count += 1;
        Ok(())
    })?;
    if count != batch.len() {
        return Err(EngineError::eval(
            "evaluator must emit exactly one result per input",
        ));
    }
    let result = if let Some(mut state) = observable {
        state.estimate.partition = options.partition_diagnostics(&raw, 2)?;
        if let Some(partition) = &mut state.estimate.partition {
            partition.training_moments = training_moments;
        }
        state.validate_stratified_samples(count as i64)?;
        state.validate_values()?;
        let accumulator = if conditional.is_some() {
            let result = super::stratification::accumulate_values(
                batch,
                &components,
                &raw,
                config,
                EvalBatchOptions {
                    require_training_values: false,
                    ..options
                },
            )?;
            let AccumulatorState::Stratified(mut stratified) = result.accumulator else {
                unreachable!()
            };
            stratified.pooled = Some(Box::new(state));
            #[cfg(feature = "gammaloop")]
            {
                stratified.histograms = histograms;
            }
            stratified.validate()?;
            AccumulatorState::Stratified(stratified)
        } else {
            AccumulatorState::Gammaloop(state)
        };
        BatchResult::new(options.feedback(feedback, &raw, 2)?, accumulator)
    } else {
        super::stratification::accumulate_values(batch, &components, &raw, config, options)?
    };
    result.validate_for_batch(batch.len(), options.require_training_values)?;
    Ok(result)
}

#[cfg(feature = "gammaloop")]
fn rescale_bundle(bundle: &mut gammalooprs::observables::ObservableSnapshotBundle, factor: f64) {
    for histogram in bundle.histograms.values_mut() {
        histogram.rescale(factor);
    }
}

#[cfg(feature = "gammaloop")]
fn validate_sample_bundle(
    bundle: &gammalooprs::observables::ObservableSnapshotBundle,
) -> Result<(), EngineError> {
    for histogram in bundle.histograms.values() {
        if histogram.sample_count != 1 {
            return Err(EngineError::eval(
                "observable output must describe exactly one sample",
            ));
        }
        for bin in histogram
            .bins
            .iter()
            .chain([&histogram.underflow_bin, &histogram.overflow_bin])
        {
            let squared = bin.sum_weights * bin.sum_weights;
            if !squared.is_finite()
                || !bin.sum_weights_squared.is_finite()
                || (squared - bin.sum_weights_squared).abs()
                    > 1e-12 * squared.max(f64::MIN_POSITIVE)
            {
                return Err(EngineError::eval(
                    "observable second moment must square the complete sample contribution",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "gammaloop"))]
mod tests {
    use super::*;
    use crate::evaluation::EvaluatorBatch;
    use crate::{Domain, Point};
    use gammalooprs::observables::{
        HistogramAccumulatorState, ObservablePhase, ObservableSnapshotBundle,
        ObservableValueTransform,
    };

    struct Fixture {
        extra: isize,
        bad_moment: bool,
    }
    impl Evaluator for Fixture {
        fn domain(&self) -> Domain {
            Domain::continuous(1)
        }
        fn eval_samples(
            &mut self,
            batch: &Batch,
            _: &[String],
            emit: &mut dyn FnMut(SampleOutput) -> Result<(), EngineError>,
        ) -> Result<(), EngineError> {
            for i in 0..(batch.len() as isize + self.extra).max(0) as usize {
                let i = batch.points()[i % batch.len()].continuous[0] as usize;
                let mut histogram = HistogramAccumulatorState::continuous(
                    "observable".into(),
                    "HwU".into(),
                    ObservablePhase::Real,
                    ObservableValueTransform::Identity,
                    0.,
                    1.,
                    false,
                    false,
                    1,
                );
                let g = (i % 7) as f64 - 2.;
                // Several correlated fills from one draw, including flow bins.
                histogram
                    .fill_continuous_sample(&[(0.5, 10.), (0.5, g - 10.), (-1., 2. * g), (2., -g)])
                    .unwrap();
                let mut histogram = histogram.snapshot();
                if self.bad_moment {
                    histogram.bins[0].sum_weights_squared += 1.;
                }
                let mut sample =
                    SampleOutput::values(vec![if i.is_multiple_of(4) { -1. } else { 1. }, 0.]);
                sample.observables = Some(ObservableSnapshotBundle {
                    histograms: [("g".into(), histogram)].into(),
                });
                emit(sample)?;
            }
            Ok(())
        }
    }
    fn fixture() -> Fixture {
        Fixture {
            extra: 0,
            bad_moment: false,
        }
    }
    fn plan() -> crate::core::StratificationPlan {
        crate::core::StratificationPlan {
            proposal_id: "observables".into(),
            partition: crate::core::StratificationConfig::Voronoi {
                domain: Domain::strata(vec![2]),
                centers: [(vec![0], vec![-1., 0.]), (vec![1], vec![1., 0.])].into(),
                scales: None,
            },
            probabilities: Some(vec![0.2, 0.8]),
            components: vec!["real".into(), "imag".into()],
        }
    }
    #[test]
    fn arbitrary_observables_flows_merges_and_exports_use_conditional_errors() {
        let plan = plan();
        let labels: Vec<_> = (0..128).map(|i| usize::from(i >= 48)).collect();
        let points: Vec<_> = labels
            .iter()
            .enumerate()
            .map(|(i, &k)| {
                Point::new(
                    vec![i as f64],
                    vec![],
                    2. / plan.probabilities.as_ref().unwrap()[k],
                )
            })
            .collect();
        let whole = fixture()
            .eval_batch(
                &Batch::new(points.clone()),
                &AccumulatorConfig::Gammaloop,
                EvalBatchOptions {
                    require_training_values: false,
                    stratification: Some((&plan, &labels)),
                },
            )
            .unwrap();
        let mut merged = AccumulatorState::Stratified(Box::new(
            super::super::accumulator::StratifiedAccumulatorState::new(
                plan.clone(),
                AccumulatorConfig::Gammaloop,
            )
            .unwrap(),
        ));
        for (points, labels) in points.chunks(11).zip(labels.chunks(11)) {
            let part = fixture()
                .eval_batch(
                    &Batch::new(points.to_vec()),
                    &AccumulatorConfig::Gammaloop,
                    EvalBatchOptions {
                        require_training_values: false,
                        stratification: Some((&plan, labels)),
                    },
                )
                .unwrap();
            merged.merge(part.accumulator).unwrap();
            // Recovery in the middle of processing preserves each population.
            merged = serde_json::from_value(serde_json::to_value(&merged).unwrap()).unwrap();
        }
        let expected: Vec<Vec<f64>> = (0..2)
            .map(|k| {
                labels
                    .iter()
                    .enumerate()
                    .filter(|(_, label)| **label == k)
                    .map(|(i, _)| {
                        if usize::from(!i.is_multiple_of(4)) == k {
                            2. * ((i % 7) as f64 - 2.)
                        } else {
                            0.
                        }
                    })
                    .collect()
            })
            .collect();
        let mean: f64 = expected
            .iter()
            .map(|v| v.iter().sum::<f64>() / v.len() as f64)
            .sum();
        let variance: f64 = expected
            .iter()
            .map(|v| {
                let n = v.len() as f64;
                let mean = v.iter().sum::<f64>() / n;
                v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n * (n - 1.))
            })
            .sum();
        for state in [&whole.accumulator, &merged] {
            let payload = crate::services::results::observable_payload(state).unwrap();
            let h = &payload["histograms"]["g"];
            assert_eq!(h["stratum_sample_counts"], serde_json::json!([48, 80]));
            for (bin, factor) in [
                (&h["bins"][0], 1.),
                (&h["underflow_bin"], 2.),
                (&h["overflow_bin"], -1.),
            ] {
                assert!((bin["estimate"]["mean"].as_f64().unwrap() - mean * factor).abs() < 1e-12);
                assert!(
                    (bin["estimate"]["error"].as_f64().unwrap() - variance.sqrt() * factor.abs())
                        .abs()
                        < 1e-12
                );
            }
            for (k, values) in expected.iter().enumerate() {
                let n = values.len() as f64;
                let s = values.iter().sum::<f64>();
                let q = values.iter().map(|v| v * v).sum::<f64>();
                let p = plan.probabilities.as_ref().unwrap()[k];
                let bin = &h["strata"][k]["bins"][0];
                assert!((bin["estimate"]["mean"].as_f64().unwrap() - s / n).abs() < 1e-12);
                assert!(
                    (bin["estimate"]["error"].as_f64().unwrap()
                        - ((q - s * s / n) / (n * (n - 1.))).sqrt())
                    .abs()
                        < 1e-12
                );
                assert!((bin["pooled"]["mean"].as_f64().unwrap() - s / (p * 128.)).abs() < 1e-12);
                assert!(
                    (bin["pooled"]["error"].as_f64().unwrap()
                        - ((q - s * s / 128.) / (128. * 127.)).sqrt() / p)
                        .abs()
                        < 1e-12
                );
                assert_eq!(
                    payload["strata"][k]["sample_count"].as_u64().unwrap(),
                    values.len() as u64
                );
            }
            let exported = crate::services::results::histogram_hwu(&payload).unwrap();
            let row: Vec<f64> = exported
                .lines()
                .find(|l| l.starts_with("  "))
                .unwrap()
                .split_whitespace()
                .map(|s| s.parse().unwrap())
                .collect();
            assert!((row[2] - mean).abs() < 1e-8);
            assert!((row[3] - variance.sqrt()).abs() < 1e-8);
            let combined = crate::services::results::combine_independent_observables(&[
                (2., state),
                (-1., state),
            ])
            .unwrap();
            assert!(
                (combined["histograms"]["g"]["bins"][0]["value"]
                    .as_f64()
                    .unwrap()
                    - mean)
                    .abs()
                    < 1e-12
            );
            assert!(
                (combined["histograms"]["g"]["bins"][0]["error"]
                    .as_f64()
                    .unwrap()
                    - (5. * variance).sqrt())
                .abs()
                    < 1e-12
            );
        }
    }
    #[test]
    fn result_count_and_sample_moments_are_enforced_before_publication() {
        let batch = Batch::new(vec![Point::new(vec![0.], vec![], 1.); 2]);
        for mut evaluator in [
            Fixture {
                extra: -1,
                bad_moment: false,
            },
            Fixture {
                extra: 1,
                bad_moment: false,
            },
            Fixture {
                extra: 0,
                bad_moment: true,
            },
        ] {
            assert!(
                evaluator
                    .eval_batch(
                        &batch,
                        &AccumulatorConfig::Gammaloop,
                        EvalBatchOptions::default()
                    )
                    .is_err()
            );
        }
        let plan = plan();
        let result = fixture()
            .eval_batch(
                &batch,
                &AccumulatorConfig::Gammaloop,
                EvalBatchOptions {
                    require_training_values: false,
                    stratification: Some((&plan, &[0, 0])),
                },
            )
            .unwrap();
        let payload = crate::services::results::observable_payload(&result.accumulator).unwrap();
        assert!(payload["histograms"]["g"]["bins"][0]["estimate"]["mean"].is_null());
        assert!(payload["histograms"]["g"]["bins"][0]["estimate"]["error"].is_null());
        assert!(crate::services::results::histogram_hwu(&payload).is_err());
    }
}
