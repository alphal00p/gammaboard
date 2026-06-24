use crate::core::{AccumulatorMetricName, AccumulatorMetricSelector, EngineError};
use crate::evaluation::{
    AccumulatorState, GammaLoopAccumulatorState, NamedScalarAccumulator, ScalarAccumulatorState,
    VectorAccumulatorState,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccumulatorMetricValue {
    pub name: AccumulatorMetricName,
    pub component: Option<String>,
    pub value: f64,
    pub uncertainty: Option<f64>,
    pub sample_count: i64,
}

pub fn extract_accumulator_metric(
    state: &AccumulatorState,
    selector: &AccumulatorMetricSelector,
) -> Result<Option<AccumulatorMetricValue>, EngineError> {
    if selector.name == AccumulatorMetricName::TimeNormalizedVariance {
        return Ok(None);
    }
    match state {
        AccumulatorState::Scalar(scalar) => {
            if selector.component.is_some() {
                return Ok(None);
            }
            Ok(Some(metric_for_scalar(scalar, selector.name, None)))
        }
        AccumulatorState::Vector(vector) => Ok(metric_for_vector(vector, selector)),
        AccumulatorState::Gammaloop(gammaloop) => Ok(metric_for_gammaloop(gammaloop, selector)),
        AccumulatorState::Empty(_) | AccumulatorState::FullVector(_) => Ok(None),
    }
}

pub fn extract_accumulator_metric_with_runtime(
    state: &AccumulatorState,
    selector: &AccumulatorMetricSelector,
    completed_samples_per_second: Option<f64>,
) -> Result<Option<AccumulatorMetricValue>, EngineError> {
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
    Ok(Some(AccumulatorMetricValue {
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
) -> Option<AccumulatorMetricValue> {
    let component = match selector.component.as_deref() {
        None => &vector.projection,
        Some("training_projection") => &vector.projection,
        Some(name) => vector.component(name)?,
    };
    Some(metric_for_named_scalar(component, selector.name))
}

fn metric_for_gammaloop(
    gammaloop: &GammaLoopAccumulatorState,
    selector: &AccumulatorMetricSelector,
) -> Option<AccumulatorMetricValue> {
    metric_for_vector(&gammaloop.estimate, selector)
}

fn metric_for_named_scalar(
    component: &NamedScalarAccumulator,
    name: AccumulatorMetricName,
) -> AccumulatorMetricValue {
    metric_for_scalar(&component.state, name, Some(component.name.clone()))
}

fn metric_for_scalar(
    state: &ScalarAccumulatorState,
    name: AccumulatorMetricName,
    component: Option<String>,
) -> AccumulatorMetricValue {
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
        AccumulatorMetricName::TimeNormalizedVariance => unreachable!(
            "time-normalized variance requires runtime throughput and is handled before scalar extraction"
        ),
    };
    let uncertainty = match name {
        AccumulatorMetricName::Mean | AccumulatorMetricName::AbsMean => Some(mean_error),
        AccumulatorMetricName::Variance => variance_error,
        AccumulatorMetricName::Error
        | AccumulatorMetricName::RelativeError
        | AccumulatorMetricName::RelativeVarianceError
        | AccumulatorMetricName::Rsd
        | AccumulatorMetricName::TimeNormalizedVariance => None,
    };
    AccumulatorMetricValue {
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
