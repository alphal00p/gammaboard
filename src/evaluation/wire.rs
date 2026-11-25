//! The evaluation contract's compact encoding. The observable model is accessed
//! only through the model boundary; scalar/vector hosts need no physics library.
use super::{EvaluationRequest, GammaLoopDiagnostics, SampleEvaluation, SampleOutput};
use crate::EngineError;
use serde::{Deserialize, Serialize};

pub const ENCODING: &str = "gammaboard-evaluation-binary-v1";
fn fail(s: impl Into<String>) -> EngineError {
    EngineError::eval(s)
}

#[derive(Default, Serialize, Deserialize)]
pub struct EvaluationResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub samples: Option<Vec<SampleOutput>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observable_schema: Option<serde_json::Value>,
}

#[derive(Default)]
pub struct Encoder {
    #[cfg(feature = "gammaloop")]
    observables: super::observable_wire::Encoder,
    schema_sent: bool,
}
impl Encoder {
    pub fn append(
        &mut self,
        sample: &SampleOutput,
        request: &EvaluationRequest,
        out: &mut Vec<u8>,
    ) -> Result<(), EngineError> {
        request.validate_sample(sample)?;
        for v in &sample.values {
            out.extend(v.to_le_bytes());
        }
        #[cfg(feature = "gammaloop")]
        if request.observables {
            self.observables
                .append_bundle(sample.observables.as_ref().unwrap(), out)?;
        }
        if request.details {
            let d = sample.evaluation.as_ref().unwrap();
            out.extend(d.integrand_value_re.to_le_bytes());
            out.extend(d.integrand_value_im.to_le_bytes());
            optional_float(out, d.parameterization_jacobian);
        }
        if request.diagnostics {
            let d = sample.diagnostics.as_ref().unwrap();
            for n in [
                d.count_total,
                d.count_double_precision,
                d.count_quad_precision,
                d.count_arb_precision,
                d.count_nan,
                d.count_nan_or_unstable,
                d.count_loop_momenta_escalated,
                d.total_generated_events,
                d.total_accepted_events,
            ] {
                out.extend(n.to_le_bytes());
            }
            for t in [
                d.total_eval_time_ms,
                d.total_integrand_eval_time_ms,
                d.total_evaluator_eval_time_ms,
                d.total_parameterization_time_ms,
                d.total_event_processing_time_ms,
            ] {
                out.extend(t.to_le_bytes());
            }
            optional_float(out, d.total_canonical_sampling_preparation_time_ms);
            optional_float(out, d.total_canonical_physical_preparation_time_ms);
        }
        Ok(())
    }
    /// Called only after a whole batch succeeds. A failed first batch must not
    /// suppress the schema on the next successful response.
    pub fn response(&mut self) -> Result<EvaluationResponse, EngineError> {
        #[cfg(feature = "gammaloop")]
        let schema = if !self.schema_sent {
            self.observables
                .schema()
                .map(serde_json::to_value)
                .transpose()
                .map_err(|e| fail(e.to_string()))?
        } else {
            None
        };
        #[cfg(not(feature = "gammaloop"))]
        let schema = None;
        self.schema_sent |= schema.is_some();
        Ok(EvaluationResponse {
            encoding: Some(ENCODING.into()),
            observable_schema: schema,
            ..Default::default()
        })
    }
}
fn optional_float(out: &mut Vec<u8>, value: Option<f64>) {
    out.push(u8::from(value.is_some()));
    if let Some(v) = value {
        out.extend(v.to_le_bytes());
    }
}

#[derive(Default)]
pub struct Decoder {
    #[cfg(feature = "gammaloop")]
    declared_schema: Option<serde_json::Value>,
    #[cfg(feature = "gammaloop")]
    observables: super::observable_wire::Decoder,
}
impl Decoder {
    pub fn with_schema(schema: Option<serde_json::Value>) -> Self {
        #[cfg(feature = "gammaloop")]
        {
            Self {
                declared_schema: schema,
                ..Default::default()
            }
        }
        #[cfg(not(feature = "gammaloop"))]
        {
            let _ = schema;
            Self::default()
        }
    }
    pub fn decode(
        &mut self,
        response: EvaluationResponse,
        bytes: &[u8],
        count: usize,
        request: &EvaluationRequest,
        emit: &mut dyn FnMut(SampleOutput) -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        if response.encoding.is_none() {
            if !bytes.is_empty() || response.observable_schema.is_some() {
                return Err(fail("unexpected binary data/schema in JSON evaluation"));
            }
            let samples = response
                .samples
                .ok_or_else(|| fail("missing evaluation samples"))?;
            if samples.len() != count {
                return Err(fail("evaluation result count mismatch"));
            }
            for sample in samples {
                request.validate_sample(&sample)?;
                emit(sample)?;
            }
            return Ok(());
        }
        if response.encoding.as_deref() != Some(ENCODING) || response.samples.is_some() {
            return Err(fail("unsupported evaluation encoding"));
        }
        #[cfg(feature = "gammaloop")]
        if request.observables || response.observable_schema.is_some() {
            if let (Some(declared), Some(actual)) =
                (&self.declared_schema, &response.observable_schema)
                && declared != actual
            {
                return Err(fail("observable schema differs from descriptor"));
            }
            let schema = response
                .observable_schema
                .or_else(|| self.declared_schema.clone());
            self.observables.install_schema(
                schema
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|e| fail(e.to_string()))?,
            )?;
            self.declared_schema = None;
        }
        #[cfg(not(feature = "gammaloop"))]
        if response.observable_schema.is_some() || request.observables {
            return Err(fail("observables require gammaloop feature"));
        }
        let mut input = Input(bytes);
        for _ in 0..count {
            let mut sample = SampleOutput::values(
                (0..request.components.len())
                    .map(|_| input.float())
                    .collect::<Result<_, _>>()?,
            );
            #[cfg(feature = "gammaloop")]
            if request.observables {
                sample.observables = Some(self.observables.read_bundle(&mut input)?);
            }
            if request.details {
                sample.evaluation = Some(SampleEvaluation {
                    integrand_value_re: input.float()?,
                    integrand_value_im: input.float()?,
                    parameterization_jacobian: input.optional_float()?,
                });
            }
            if request.diagnostics {
                sample.diagnostics = Some(GammaLoopDiagnostics {
                    count_total: input.counter()?,
                    count_double_precision: input.counter()?,
                    count_quad_precision: input.counter()?,
                    count_arb_precision: input.counter()?,
                    count_nan: input.counter()?,
                    count_nan_or_unstable: input.counter()?,
                    count_loop_momenta_escalated: input.counter()?,
                    total_generated_events: input.counter()?,
                    total_accepted_events: input.counter()?,
                    total_eval_time_ms: input.float()?,
                    total_integrand_eval_time_ms: input.float()?,
                    total_evaluator_eval_time_ms: input.float()?,
                    total_parameterization_time_ms: input.float()?,
                    total_event_processing_time_ms: input.float()?,
                    total_canonical_sampling_preparation_time_ms: input.optional_float()?,
                    total_canonical_physical_preparation_time_ms: input.optional_float()?,
                });
            }
            request.validate_sample(&sample)?;
            emit(sample)?;
        }
        if !input.0.is_empty() {
            return Err(fail("trailing evaluation binary data"));
        }
        Ok(())
    }
}

pub(crate) struct Input<'a>(pub(crate) &'a [u8]);
impl Input<'_> {
    fn word<const N: usize>(&mut self) -> Result<[u8; N], EngineError> {
        let word = self
            .0
            .get(..N)
            .ok_or_else(|| fail("truncated evaluation binary data"))?
            .try_into()
            .unwrap();
        self.0 = &self.0[N..];
        Ok(word)
    }
    #[cfg(feature = "gammaloop")]
    pub(crate) fn index(&mut self) -> Result<usize, EngineError> {
        Ok(u32::from_le_bytes(self.word()?) as usize)
    }
    #[cfg(feature = "gammaloop")]
    pub(crate) fn count(&mut self) -> Result<usize, EngineError> {
        usize::try_from(u64::from_le_bytes(self.word()?))
            .map_err(|_| fail("observable counter overflow"))
    }
    fn counter(&mut self) -> Result<i64, EngineError> {
        let v = i64::from_le_bytes(self.word()?);
        if v < 0 {
            return Err(fail("negative diagnostic counter"));
        }
        Ok(v)
    }
    pub(crate) fn float(&mut self) -> Result<f64, EngineError> {
        let v = f64::from_le_bytes(self.word()?);
        if !v.is_finite() {
            return Err(fail("non-finite evaluation binary value"));
        }
        Ok(v)
    }
    fn optional_float(&mut self) -> Result<Option<f64>, EngineError> {
        match self.word::<1>()?[0] {
            0 => Ok(None),
            1 => self.float().map(Some),
            _ => Err(fail("invalid optional value tag")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (EvaluationRequest, SampleOutput) {
        let request = EvaluationRequest {
            components: vec!["imag".into(), "real".into()],
            observables: false,
            details: true,
            diagnostics: true,
        };
        let mut sample = SampleOutput::values(vec![-4., 6.]);
        sample.evaluation = Some(SampleEvaluation {
            integrand_value_re: 2.,
            integrand_value_im: -4. / 3.,
            parameterization_jacobian: Some(3.),
        });
        sample.diagnostics = Some(GammaLoopDiagnostics {
            count_total: 1,
            count_double_precision: 1,
            total_generated_events: 3,
            total_accepted_events: 2,
            total_eval_time_ms: 1.25,
            total_canonical_sampling_preparation_time_ms: Some(0.),
            ..Default::default()
        });
        (request, sample)
    }
    #[test]
    fn selected_binary_and_json_preserve_details_and_missing_optional_timings() {
        let (request, sample) = fixture();
        let expected = serde_json::to_value(&sample).unwrap();
        let mut encoder = Encoder::default();
        let mut bytes = Vec::new();
        encoder.append(&sample, &request, &mut bytes).unwrap();
        let mut decoder = Decoder::default();
        let mut check = |row| {
            assert_eq!(serde_json::to_value(row).unwrap(), expected);
            Ok(())
        };
        decoder
            .decode(encoder.response().unwrap(), &bytes, 1, &request, &mut check)
            .unwrap();
        decoder
            .decode(
                EvaluationResponse {
                    samples: Some(vec![sample]),
                    ..Default::default()
                },
                &[],
                1,
                &request,
                &mut check,
            )
            .unwrap();
        // Output selection can change on the same worker/connection.
        let values = EvaluationRequest::values(vec!["real".into()]);
        let mut bytes = Vec::new();
        encoder
            .append(&SampleOutput::values(vec![6.]), &values, &mut bytes)
            .unwrap();
        decoder
            .decode(
                encoder.response().unwrap(),
                &bytes,
                1,
                &values,
                &mut |row| {
                    assert!(row.diagnostics.is_none());
                    assert!(row.evaluation.is_none());
                    Ok(())
                },
            )
            .unwrap();
    }
    #[test]
    fn malformed_or_missing_channels_fail() {
        let (request, sample) = fixture();
        let mut encoder = Encoder::default();
        let mut bytes = Vec::new();
        encoder.append(&sample, &request, &mut bytes).unwrap();
        for n in 0..bytes.len() {
            assert!(
                Decoder::default()
                    .decode(
                        encoder.response().unwrap(),
                        &bytes[..n],
                        1,
                        &request,
                        &mut |_| Ok(())
                    )
                    .is_err()
            );
        }
        bytes.push(0);
        assert!(
            Decoder::default()
                .decode(
                    encoder.response().unwrap(),
                    &bytes,
                    1,
                    &request,
                    &mut |_| Ok(())
                )
                .is_err()
        );
        assert!(
            request
                .validate_sample(&SampleOutput::values(vec![1., 2.]))
                .is_err()
        );
        let (_, mut sample) = fixture();
        sample.diagnostics.as_mut().unwrap().count_total = 0;
        assert!(request.validate_sample(&sample).is_err());
        sample.diagnostics.as_mut().unwrap().count_total = 1;
        sample.diagnostics.as_mut().unwrap().total_eval_time_ms = -1.;
        assert!(request.validate_sample(&sample).is_err());
    }
}

#[cfg(all(test, feature = "gammaloop"))]
mod schema_tests {
    use super::*;
    #[test]
    fn discovery_schema_can_supply_first_binary_observable_response() {
        let schema = serde_json::json!({"histograms":{}});
        let request = EvaluationRequest {
            observables: true,
            ..EvaluationRequest::values(vec!["real".into()])
        };
        let mut decoder = Decoder::with_schema(Some(schema.clone()));
        for _ in 0..2 {
            decoder
                .decode(
                    EvaluationResponse {
                        encoding: Some(ENCODING.into()),
                        ..Default::default()
                    },
                    &2_f64.to_le_bytes(),
                    1,
                    &request,
                    &mut |sample| {
                        assert_eq!(serde_json::to_value(sample.observables).unwrap(), schema);
                        Ok(())
                    },
                )
                .unwrap();
        }
    }
}
