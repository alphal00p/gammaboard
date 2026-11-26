//! Normalization metadata for a diagnostic slice of the training projection.
use crate::core::{AccumulatorConfig, AccumulatorMetricName, AccumulatorMetricSelector};
use crate::evaluation::{AccumulatorState, extract_accumulator_metric};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfProjection {
    pub key: String,
    pub label: String,
    pub nonnegative: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfReference {
    pub value: f64,
    pub error: Option<f64>,
    pub sample_count: Option<i64>,
    pub source: String,
    pub task_id: Option<i64>,
    pub task_name: Option<String>,
}

pub fn projection(state: &AccumulatorState) -> Option<PdfProjection> {
    use crate::core::TrainingProjection;
    let gamma = match state {
        AccumulatorState::Gammaloop(s) => Some(s),
        AccumulatorState::Stratified(s) => s.pooled.as_deref(),
        _ => None,
    };
    if let Some(gamma) = gamma {
        use crate::evaluation::evaluator::gammaloop::TrainingProjection as Phase;
        let (key, label, nonnegative) = match gamma.training_projection? {
            Phase::Real => ("real", "Re(f)", false),
            Phase::Imag => ("imag", "Im(f)", false),
            Phase::Abs => ("norm", "|f|", true),
            Phase::AbsSq => ("norm_squared", "|f|²", true),
        };
        return Some(PdfProjection {
            key: key.into(),
            label: label.into(),
            nonnegative,
        });
    }
    match state.config() {
        AccumulatorConfig::Vector {
            training_projection: TrainingProjection::Norm,
            ..
        } => Some(PdfProjection {
            key: "norm".into(),
            label: "‖f‖₂".into(),
            nonnegative: true,
        }),
        AccumulatorConfig::Vector {
            training_projection: TrainingProjection::Component { name },
            ..
        } => Some(PdfProjection {
            key: name.clone(),
            label: name,
            nonnegative: false,
        }),
        _ => None,
    }
}

pub fn reference(state: &AccumulatorState) -> Option<PdfReference> {
    // Never guess the phase of an old GammaLoop checkpoint.
    let projection = projection(state)?;
    let result = extract_accumulator_metric(
        state,
        &AccumulatorMetricSelector {
            name: if projection.nonnegative {
                AccumulatorMetricName::Mean
            } else {
                AccumulatorMetricName::AbsMean
            },
            component: Some("training_projection".into()),
        },
    )
    .ok()??;
    (result.value.is_finite() && result.value > 0.0).then_some(PdfReference {
        value: result.value,
        error: result.uncertainty,
        sample_count: Some(result.sample_count),
        source: "sampler_source".into(),
        task_id: None,
        task_name: None,
    })
}

pub fn configured_reference(
    projection: &PdfProjection,
    target: Option<&serde_json::Value>,
    error: Option<&serde_json::Value>,
) -> Option<PdfReference> {
    let target = crate::core::target::RunTarget::parse(target, error).ok()??;
    // Signed component targets cannot normalize their absolute values.
    let absolute_key = format!("abs_{}", projection.key);
    let value = target
        .components
        .get("abs_training_projection")
        .or_else(|| target.components.get(&absolute_key))
        .or_else(|| {
            projection
                .nonnegative
                .then(|| target.components.get(&projection.key))
                .flatten()
        })?;
    (value.value > 0.0).then_some(PdfReference {
        value: value.value,
        error: Some(value.error),
        sample_count: None,
        source: "configured_target".into(),
        task_id: None,
        task_name: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn pdf_reference_follows_training_phase_and_absolute_uncertainty() {
        use crate::evaluation::GammaLoopAccumulatorState;
        use crate::evaluation::evaluator::gammaloop::TrainingProjection;
        let mut state = GammaLoopAccumulatorState::default();
        for x in [-1.0, 3.0] {
            state
                .estimate
                .ingest_vector(&[x, 10.0], &crate::Point::new(vec![], vec![], 1.0))
                .unwrap();
        }
        assert!(reference(&AccumulatorState::Gammaloop(state.clone())).is_none());
        state.training_projection = Some(TrainingProjection::Real);
        let r = reference(&AccumulatorState::Gammaloop(state.clone())).unwrap();
        assert_eq!(
            (r.value, r.error, r.sample_count),
            (2.0, Some(1.0), Some(2))
        );
        state.training_projection = Some(TrainingProjection::Imag);
        let r = reference(&AccumulatorState::Gammaloop(state)).unwrap();
        assert_eq!((r.value, r.error), (10.0, Some(0.0)));
    }

    #[test]
    fn signed_targets_do_not_normalize_absolute_projections() {
        let p = PdfProjection {
            key: "real".into(),
            label: "Re(f)".into(),
            nonnegative: false,
        };
        assert!(configured_reference(&p, Some(&json!(2.0)), None).is_none());
        let r = configured_reference(
            &p,
            Some(&json!({"components":{"abs_real":3.0}})),
            Some(&json!({"abs_real":0.2})),
        )
        .unwrap();
        assert_eq!((r.value, r.error), (3.0, Some(0.2)));
        assert!(
            configured_reference(&p, Some(&json!({"components":{"abs_real":0.0}})), None).is_none()
        );
    }
}
