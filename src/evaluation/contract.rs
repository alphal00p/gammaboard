//! Transport-independent evaluator capabilities and per-call requirements.
use super::{Batch, Evaluator, ObservableTrainingProjection, SampleOutput};
use crate::{AccumulatorConfig, Domain, EngineError};
use serde::{Deserialize, Serialize};

pub const EVALUATION_PROTOCOL: &str = "gammaboard-evaluation-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EvaluatorDescriptor {
    pub domain: Domain,
    pub components: Vec<String>,
    #[serde(default)]
    pub observables: bool,
    #[serde(default)]
    pub details: bool,
    #[serde(default)]
    pub diagnostics: bool,
    /// Compatibility projection for scalar tasks requesting the synthetic `value`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scalar_projection: Option<ObservableTrainingProjection>,
    /// Optional discovery-time schema; the binary stream also establishes it lazily.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observable_schema: Option<serde_json::Value>,
}

impl EvaluatorDescriptor {
    pub fn values(domain: Domain, components: Vec<String>) -> Self {
        Self {
            domain,
            components,
            observables: false,
            details: false,
            diagnostics: false,
            scalar_projection: None,
            observable_schema: None,
        }
    }
    pub fn complex(domain: Domain, projection: ObservableTrainingProjection) -> Self {
        Self {
            scalar_projection: Some(projection),
            ..Self::values(domain, vec!["real".into(), "imag".into()])
        }
    }
    pub fn validate(&self) -> Result<(), EngineError> {
        self.domain.validate().map_err(EngineError::build)?;
        if self.components.is_empty()
            || self.components.iter().any(|v| v.trim().is_empty())
            || self
                .components
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.components.len()
        {
            return Err(EngineError::build(
                "descriptor components must be nonempty and unique",
            ));
        }
        if self.scalar_projection.is_some()
            && !["real", "imag"]
                .iter()
                .all(|v| self.components.iter().any(|c| c == v))
        {
            return Err(EngineError::build(
                "complex scalar projection requires real/imag components",
            ));
        }
        Ok(())
    }
    pub fn request(&self, components: Vec<String>, observables: bool) -> EvaluationRequest {
        EvaluationRequest {
            components,
            observables,
            details: observables && self.details,
            diagnostics: observables && self.diagnostics,
        }
    }
    pub fn plan(&self, accumulator: &AccumulatorConfig) -> Result<EvaluationRequest, EngineError> {
        let observables = matches!(accumulator, AccumulatorConfig::Gammaloop);
        let components = if observables {
            vec!["real".into(), "imag".into()]
        } else {
            accumulator
                .components()
                .unwrap_or(&["value".into()])
                .to_vec()
        };
        let request = self.request(components, observables);
        canonical_request(self, &request)?;
        Ok(request)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EvaluationRequest {
    pub components: Vec<String>,
    #[serde(default)]
    pub observables: bool,
    #[serde(default)]
    pub details: bool,
    #[serde(default)]
    pub diagnostics: bool,
}
impl EvaluationRequest {
    pub fn values(components: Vec<String>) -> Self {
        Self {
            components,
            observables: false,
            details: false,
            diagnostics: false,
        }
    }
    pub fn validate(&self, descriptor: &EvaluatorDescriptor) -> Result<(), EngineError> {
        if self.components.is_empty()
            || self
                .components
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.components.len()
            || self
                .components
                .iter()
                .any(|c| !descriptor.components.contains(c))
        {
            return Err(EngineError::invalid_input(format!(
                "requested components {:?} are not supported by {:?}",
                self.components, descriptor.components
            )));
        }
        for (requested, supported, name) in [
            (self.observables, descriptor.observables, "observables"),
            (self.details, descriptor.details, "details"),
            (self.diagnostics, descriptor.diagnostics, "diagnostics"),
        ] {
            if requested && !supported {
                return Err(EngineError::invalid_input(format!(
                    "evaluator does not provide requested {name}"
                )));
            }
        }
        Ok(())
    }
    pub fn validate_sample(&self, sample: &SampleOutput) -> Result<(), EngineError> {
        if sample.values.len() != self.components.len()
            || sample.values.iter().any(|v| !v.is_finite())
        {
            return Err(EngineError::eval("invalid evaluation value row"));
        }
        if self.details && sample.evaluation.is_none()
            || self.diagnostics && sample.diagnostics.is_none()
        {
            return Err(EngineError::eval(
                "missing requested evaluation details or diagnostics",
            ));
        }
        #[cfg(feature = "gammaloop")]
        if self.observables && sample.observables.is_none() {
            return Err(EngineError::eval("missing requested observables"));
        }
        #[cfg(not(feature = "gammaloop"))]
        if self.observables {
            return Err(EngineError::eval(
                "observables currently require the gammaloop feature",
            ));
        }
        if let Some(detail) = &sample.evaluation
            && (!detail.integrand_value_re.is_finite()
                || !detail.integrand_value_im.is_finite()
                || detail
                    .parameterization_jacobian
                    .is_some_and(|v| !v.is_finite()))
        {
            return Err(EngineError::eval("non-finite evaluation details"));
        }
        if let Some(d) = &sample.diagnostics {
            if d.count_total != 1
                || [
                    d.count_double_precision,
                    d.count_quad_precision,
                    d.count_arb_precision,
                    d.count_nan,
                    d.count_nan_or_unstable,
                    d.count_loop_momenta_escalated,
                    d.total_generated_events,
                    d.total_accepted_events,
                ]
                .iter()
                .any(|&n| n < 0)
            {
                return Err(EngineError::eval(
                    "diagnostics must describe one sample with nonnegative counters",
                ));
            }
            if [
                Some(d.total_eval_time_ms),
                Some(d.total_integrand_eval_time_ms),
                Some(d.total_evaluator_eval_time_ms),
                Some(d.total_parameterization_time_ms),
                Some(d.total_event_processing_time_ms),
                d.total_canonical_sampling_preparation_time_ms,
                d.total_canonical_physical_preparation_time_ms,
            ]
            .into_iter()
            .flatten()
            .any(|t| !t.is_finite() || t < 0.)
            {
                return Err(EngineError::eval(
                    "diagnostic timings must be finite and nonnegative",
                ));
            }
        }
        Ok(())
    }
}

fn canonical_request(
    descriptor: &EvaluatorDescriptor,
    request: &EvaluationRequest,
) -> Result<EvaluationRequest, EngineError> {
    let mut canonical = request.clone();
    if !descriptor.components.iter().any(|c| c == "value")
        && request.components.iter().any(|c| c == "value")
        && descriptor.scalar_projection.is_some()
    {
        canonical.components.retain(|c| c != "value");
        for name in ["real", "imag"] {
            if !canonical.components.iter().any(|c| c == name) {
                canonical.components.push(name.into());
            }
        }
    }
    canonical.validate(descriptor)?;
    Ok(canonical)
}

/// Shared task/configuration projection; neither process workers nor native
/// backends apply outer weights or implement their own scalar-task semantics.
pub fn evaluate_requested<T: Evaluator + ?Sized>(
    evaluator: &mut T,
    batch: &Batch,
    request: &EvaluationRequest,
    emit: &mut dyn FnMut(SampleOutput) -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    let descriptor = evaluator.descriptor();
    let canonical = canonical_request(&descriptor, request)?;
    let projection = descriptor.scalar_projection;
    let remap = canonical.components != request.components;
    let mut count = 0;
    evaluator.evaluate(batch, &canonical, &mut |mut sample| {
        count += 1;
        if count > batch.len() {
            return Err(EngineError::eval("too many evaluation outputs"));
        }
        canonical.validate_sample(&sample)?;
        if remap {
            let re = canonical
                .components
                .iter()
                .position(|c| c == "real")
                .unwrap();
            let im = canonical
                .components
                .iter()
                .position(|c| c == "imag")
                .unwrap();
            sample.values = request
                .components
                .iter()
                .map(|name| {
                    if name == "value" {
                        projection
                            .unwrap()
                            .project(num::Complex::new(sample.values[re], sample.values[im]))
                    } else {
                        sample.values[canonical.components.iter().position(|c| c == name).unwrap()]
                    }
                })
                .collect();
        }
        request.validate_sample(&sample)?;
        emit(sample)
    })?;
    if count != batch.len() {
        return Err(EngineError::eval("missing evaluation outputs"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        rows: usize,
    }
    impl Evaluator for Fixture {
        fn descriptor(&self) -> EvaluatorDescriptor {
            let mut d = EvaluatorDescriptor::complex(
                Domain::continuous(1),
                ObservableTrainingProjection::Real,
            );
            d.details = true;
            d
        }
        fn evaluate(
            &mut self,
            _: &Batch,
            request: &EvaluationRequest,
            emit: &mut dyn FnMut(SampleOutput) -> Result<(), EngineError>,
        ) -> Result<(), EngineError> {
            for _ in 0..self.rows {
                let mut sample = SampleOutput::values(
                    request
                        .components
                        .iter()
                        .map(|c| if c == "real" { 6. } else { 0. })
                        .collect(),
                );
                if request.details {
                    sample.evaluation = Some(super::super::SampleEvaluation {
                        integrand_value_re: 2.,
                        integrand_value_im: 0.,
                        parameterization_jacobian: Some(3.),
                    });
                }
                emit(sample)?;
            }
            Ok(())
        }
    }
    #[test]
    fn central_projection_does_not_apply_informational_jacobian_again() {
        use crate::{EvalBatchOptions, EvaluatorBatch, Point};
        let batch = Batch::new(vec![Point::new(vec![0.5], vec![], 5.)]);
        let request = EvaluationRequest {
            components: vec!["value".into(), "imag".into()],
            observables: false,
            details: true,
            diagnostics: false,
        };
        evaluate_requested(&mut Fixture { rows: 1 }, &batch, &request, &mut |s| {
            assert_eq!(s.values, vec![6., 0.]);
            assert_eq!(s.evaluation.unwrap().parameterization_jacobian, Some(3.));
            Ok(())
        })
        .unwrap();
        let result = Fixture { rows: 1 }
            .eval_batch(
                &batch,
                &AccumulatorConfig::FullVector {
                    components: vec!["value".into()],
                },
                EvalBatchOptions::default(),
            )
            .unwrap();
        let crate::AccumulatorState::FullVector(state) = result.accumulator else {
            panic!()
        };
        assert_eq!(state.values_row_major, vec![30.]);
        for rows in [0, 2] {
            assert!(
                evaluate_requested(&mut Fixture { rows }, &batch, &request, &mut |_| Ok(()))
                    .is_err()
            );
        }
    }
}
