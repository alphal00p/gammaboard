use crate::core::MeasurementResult;
use crate::core::{AccumulatorMetricName, AccumulatorMetricSelector, EngineError};
use crate::evaluation::{
    AccumulatorState, GammaLoopAccumulatorState, NamedScalarAccumulator, ScalarAccumulatorState,
    VectorAccumulatorState,
};

pub fn extract_accumulator_metric(
    state: &AccumulatorState,
    selector: &AccumulatorMetricSelector,
) -> Result<Option<MeasurementResult>, EngineError> {
    if selector.name == AccumulatorMetricName::TimeNormalizedVariance {
        return Ok(None);
    }
    match state {
        AccumulatorState::Vector(vector) => Ok(metric_for_vector(vector, selector)),
        AccumulatorState::Gammaloop(gammaloop) => Ok(metric_for_gammaloop(gammaloop, selector)),
        AccumulatorState::Empty(_) | AccumulatorState::FullVector(_) => Ok(None),
    }
}

pub fn extract_accumulator_metric_with_runtime(
    state: &AccumulatorState,
    selector: &AccumulatorMetricSelector,
    completed_samples_per_second: Option<f64>,
) -> Result<Option<MeasurementResult>, EngineError> {
    if selector.name != AccumulatorMetricName::TimeNormalizedVariance {
        return extract_accumulator_metric(state, selector);
    }
    let Some(rate) = completed_samples_per_second.filter(|value| value.is_finite() && *value > 0.0)
    else {
        return Ok(None);
    };
    let variance_selector = AccumulatorMetricSelector {
        name: AccumulatorMetricName::Variance,
        component: selector.component.clone(),
    };
    let Some(variance) = extract_accumulator_metric(state, &variance_selector)? else {
        return Ok(None);
    };
    Ok(Some(MeasurementResult {
        name: AccumulatorMetricName::TimeNormalizedVariance,
        component: variance.component,
        value: variance.value / rate,
        uncertainty: variance.uncertainty.map(|uncertainty| uncertainty / rate),
        sample_count: variance.sample_count,
    }))
}

fn metric_for_vector(
    vector: &VectorAccumulatorState,
    selector: &AccumulatorMetricSelector,
) -> Option<MeasurementResult> {
    match selector.component.as_deref() {
        None | Some("training_projection") => Some(metric_for_scalar(
            &vector.projection().state,
            selector.name,
            Some("training_projection".into()),
        )),
        Some(name) => Some(metric_for_named_scalar(
            vector.component(name)?,
            selector.name,
        )),
    }
}

fn metric_for_gammaloop(
    gammaloop: &GammaLoopAccumulatorState,
    selector: &AccumulatorMetricSelector,
) -> Option<MeasurementResult> {
    match selector.component.as_deref() {
        Some("norm") => Some(metric_for_scalar(
            gammaloop.norm_statistics(),
            selector.name,
            Some("norm".into()),
        )),
        None | Some("training_projection") => Some(metric_for_scalar(
            gammaloop.training_statistics()?,
            selector.name,
            Some("training_projection".to_string()),
        )),
        _ => metric_for_vector(&gammaloop.estimate, selector),
    }
}

fn metric_for_named_scalar(
    component: &NamedScalarAccumulator,
    name: AccumulatorMetricName,
) -> MeasurementResult {
    metric_for_scalar(&component.state, name, Some(component.name.clone()))
}

fn metric_for_scalar(
    state: &ScalarAccumulatorState,
    name: AccumulatorMetricName,
    component: Option<String>,
) -> MeasurementResult {
    let mean = state.mean();
    let mean_error = state.stderr();
    let variance = state.variance();
    let variance_error = state.variance_stderr();
    let value = match name {
        AccumulatorMetricName::Mean => mean,
        AccumulatorMetricName::AbsMean => state.mean_abs(),
        AccumulatorMetricName::Error => mean_error,
        AccumulatorMetricName::RelativeError => relative_error(mean, mean_error),
        AccumulatorMetricName::Variance => variance,
        AccumulatorMetricName::RelativeVarianceError => variance_error
            .map(|error| relative_error(variance, error))
            .unwrap_or(f64::INFINITY),
        AccumulatorMetricName::Rsd => state.rsd(),
        AccumulatorMetricName::Ess => state.ess(),
        AccumulatorMetricName::TimeNormalizedVariance => unreachable!(
            "time-normalized variance requires runtime throughput and is handled before scalar extraction"
        ),
    };
    let uncertainty = match name {
        AccumulatorMetricName::Mean => Some(mean_error),
        AccumulatorMetricName::AbsMean => Some(state.mean_abs_stderr()),
        AccumulatorMetricName::Variance => variance_error,
        AccumulatorMetricName::Rsd => state.rsd_stderr(),
        AccumulatorMetricName::Error
        | AccumulatorMetricName::RelativeError
        | AccumulatorMetricName::RelativeVarianceError
        | AccumulatorMetricName::Ess
        | AccumulatorMetricName::TimeNormalizedVariance => None,
    };
    MeasurementResult {
        name,
        component,
        value,
        uncertainty,
        sample_count: state.count,
    }
}

pub fn relative_error(value: f64, abs_error: f64) -> f64 {
    let denominator = value.abs();
    if denominator == 0.0 {
        if abs_error == 0.0 { 0.0 } else { f64::INFINITY }
    } else {
        abs_error.abs() / denominator
    }
}

/// One estimate calculation shared by execution and dashboard ETA. GammaLoop's
/// default is mean per-sample magnitude, independent of the training projection.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProjectedEstimate {
    pub value: f64,
    pub error: f64,
}

pub(crate) fn estimate_for_stop(
    state: &AccumulatorState,
    selector: Option<&AccumulatorMetricSelector>,
    rate: Option<f64>,
) -> Result<Option<ProjectedEstimate>, EngineError> {
    let default = AccumulatorMetricSelector {
        name: AccumulatorMetricName::Mean,
        component: matches!(state, AccumulatorState::Gammaloop(_)).then(|| "norm".into()),
    };
    Ok(
        extract_accumulator_metric_with_runtime(state, selector.unwrap_or(&default), rate)?
            .and_then(|metric| {
                metric.uncertainty.map(|error| ProjectedEstimate {
                    value: metric.value,
                    error,
                })
            }),
    )
}

#[cfg(test)]
mod stopping_tests {
    use super::*;
    use crate::Point;

    #[test]
    fn gammaloop_norm_stopping_is_independent_of_training_and_cancellation() {
        let mut state = GammaLoopAccumulatorState::default();
        state
            .estimate
            .ingest_vector(&[1.0, 0.0], &Point::new(vec![], vec![], 1.0))
            .unwrap();
        state
            .estimate
            .ingest_vector(&[-1.0, 0.0], &Point::new(vec![], vec![], 1.0))
            .unwrap();
        state.training_projection =
            Some(crate::evaluation::evaluator::gammaloop::TrainingProjection::Real);
        let accumulator = AccumulatorState::Gammaloop(state);
        let norm = estimate_for_stop(&accumulator, None, None)
            .unwrap()
            .unwrap();
        let real = estimate_for_stop(
            &accumulator,
            Some(&AccumulatorMetricSelector {
                name: AccumulatorMetricName::Mean,
                component: Some("real".into()),
            }),
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(norm.value, 1.0);
        assert_eq!(norm.error, 0.0);
        assert_eq!(real.value, 0.0);
        assert!(real.error > 0.0);
    }
}
