//! Negotiated, lossless per-sample observable transport. Static layout is sent
//! once; numerical rows remain unweighted until the host reducer consumes them.
use crate::evaluation::observables::{HistogramBinSnapshot, ObservableSnapshotBundle};
use crate::{EngineError, GammaLoopAccumulatorState, SampleOutput};
use serde::{Deserialize, Serialize};

pub const ENCODING: &str = "gammaboard-observables-binary-v1";

#[derive(Default, Serialize, Deserialize)]
pub struct ObservableResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observables: Option<Vec<ObservableSnapshotBundle>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observable_encoding: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observable_schema: Option<ObservableSnapshotBundle>,
}

#[derive(Default)]
pub struct Encoder {
    schema: Option<ObservableSnapshotBundle>,
}

fn fail(message: impl Into<String>) -> EngineError {
    EngineError::eval(message)
}

impl Encoder {
    pub fn schema(&self) -> Option<&ObservableSnapshotBundle> {
        self.schema.as_ref()
    }

    pub fn append(
        &mut self,
        values: &[f64],
        bundle: &ObservableSnapshotBundle,
        bytes: &mut Vec<u8>,
    ) -> Result<(), EngineError> {
        if values.len() != 2 || values.iter().any(|x| !x.is_finite()) {
            return Err(fail("binary observables require finite real/imag values"));
        }
        for value in values {
            bytes.extend(value.to_le_bytes());
        }
        self.append_bundle(bundle, bytes)
    }

    pub(crate) fn append_bundle(
        &mut self,
        bundle: &ObservableSnapshotBundle,
        bytes: &mut Vec<u8>,
    ) -> Result<(), EngineError> {
        if bundle.histograms.values().any(|h| h.sample_count != 1) {
            return Err(fail("observable bundle must describe exactly one sample"));
        }
        let schema = GammaLoopAccumulatorState::cleared_bundle(bundle);
        match &self.schema {
            Some(previous) if previous != &schema => {
                return Err(fail(
                    "observable schema changed; use JSON output for changing layouts",
                ));
            }
            None => self.schema = Some(schema),
            _ => {}
        }
        for histogram in bundle.histograms.values() {
            for count in [
                histogram.statistics.in_range_entry_count,
                histogram.statistics.nan_value_count,
                histogram.statistics.mitigated_pair_count,
            ] {
                bytes.extend((count as u64).to_le_bytes());
            }
            let bins = histogram
                .bins
                .iter()
                .chain([&histogram.underflow_bin, &histogram.overflow_bin]);
            let populated_count = bins.clone().filter(|bin| populated(bin)).count();
            bytes.extend(
                u32::try_from(populated_count)
                    .map_err(|_| fail("too many observable bins"))?
                    .to_le_bytes(),
            );
            for (index, bin) in bins.enumerate().filter(|(_, bin)| populated(bin)) {
                if !bin.sum_weights.is_finite() || !bin.sum_weights_squared.is_finite() {
                    return Err(fail("non-finite observable bin"));
                }
                bytes.extend(
                    u32::try_from(index)
                        .map_err(|_| fail("observable bin index overflow"))?
                        .to_le_bytes(),
                );
                bytes.extend((bin.entry_count as u64).to_le_bytes());
                bytes.extend(bin.sum_weights.to_le_bytes());
                bytes.extend(bin.sum_weights_squared.to_le_bytes());
                bytes.extend((bin.mitigated_fill_count as u64).to_le_bytes());
            }
        }
        Ok(())
    }
}

fn populated(bin: &HistogramBinSnapshot) -> bool {
    bin.entry_count != 0
        || bin.mitigated_fill_count != 0
        || bin.sum_weights.to_bits() != 0
        || bin.sum_weights_squared.to_bits() != 0
}

#[derive(Default)]
pub struct Decoder {
    schema: Option<ObservableSnapshotBundle>,
}

impl Decoder {
    pub fn decode(
        &mut self,
        response: ObservableResponse,
        bytes: &[u8],
        samples: usize,
        emit: &mut dyn FnMut(SampleOutput) -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        if response.observable_encoding.as_deref() != Some(ENCODING)
            || response.observables.is_some()
        {
            return Err(fail("invalid binary observable response encoding"));
        }
        self.install_schema(response.observable_schema)?;
        if samples == 0 {
            return if bytes.is_empty() {
                Ok(())
            } else {
                Err(fail("unexpected binary data for empty observable batch"))
            };
        }
        let mut input = Input(bytes);
        for _ in 0..samples {
            let mut output = SampleOutput::values(vec![input.float()?, input.float()?]);
            let bundle = self.read_bundle(&mut input)?;
            output.observables = Some(bundle);
            emit(output)?;
        }
        if !input.0.is_empty() {
            return Err(fail("trailing observable binary data"));
        }
        Ok(())
    }
    pub(crate) fn install_schema(
        &mut self,
        schema: Option<ObservableSnapshotBundle>,
    ) -> Result<(), EngineError> {
        if let Some(schema) = schema {
            if self.schema.is_some() {
                return Err(fail("observable schema was sent more than once"));
            }
            if GammaLoopAccumulatorState::cleared_bundle(&schema) != schema {
                return Err(fail(
                    "observable schema must have zero counters and moments",
                ));
            }
            self.schema = Some(schema);
        }
        Ok(())
    }
    pub(crate) fn read_bundle(
        &self,
        input: &mut Input<'_>,
    ) -> Result<ObservableSnapshotBundle, EngineError> {
        let schema = self
            .schema
            .as_ref()
            .ok_or_else(|| fail("missing observable schema"))?;
        let mut bundle = schema.clone();
        for histogram in bundle.histograms.values_mut() {
            histogram.sample_count = 1;
            histogram.statistics.in_range_entry_count = input.count()?;
            histogram.statistics.nan_value_count = input.count()?;
            histogram.statistics.mitigated_pair_count = input.count()?;
            let count = input.index()?;
            let bin_count = histogram.bins.len() + 2;
            if count > bin_count {
                return Err(fail("too many sparse observable bins"));
            }
            let mut previous = None;
            for _ in 0..count {
                let index = input.index()?;
                if index >= bin_count || previous.is_some_and(|p| p >= index) {
                    return Err(fail(
                        "observable bin indices must be in range and strictly increasing",
                    ));
                }
                previous = Some(index);
                let bin = if index < histogram.bins.len() {
                    &mut histogram.bins[index]
                } else if index == histogram.bins.len() {
                    &mut histogram.underflow_bin
                } else {
                    &mut histogram.overflow_bin
                };
                bin.entry_count = input.count()?;
                bin.sum_weights = input.float()?;
                bin.sum_weights_squared = input.float()?;
                bin.mitigated_fill_count = input.count()?;
            }
        }
        Ok(bundle)
    }
}

use super::wire::Input;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> ObservableSnapshotBundle {
        let bin = json!({"x_min":0.,"x_max":1.,"bin_id":null,"label":null,"entry_count":2,
            "sum_weights":-3.,"sum_weights_squared":9.,"mitigated_fill_count":1});
        serde_json::from_value(json!({"histograms":{"h":{
            "kind":"continuous","title":"Example","type_description":"AL","phase":"real",
            "value_transform":"identity","supports_misbinning_mitigation":true,"x_min":0.,"x_max":1.,
            "sample_count":1,"log_x_axis":false,"log_y_axis":true,"discrete_min_bin_id":null,"discrete_ordering":null,
            "bins":[bin],"underflow_bin":bin,"overflow_bin":bin,
            "statistics":{"in_range_entry_count":2,"nan_value_count":1,"mitigated_pair_count":3}
        }}})).unwrap()
    }
    fn header(schema: Option<ObservableSnapshotBundle>) -> ObservableResponse {
        ObservableResponse {
            observable_encoding: Some(ENCODING.into()),
            observable_schema: schema,
            ..Default::default()
        }
    }

    #[test]
    fn sparse_rows_preserve_counts_flow_bins_correlated_moments_and_empty_samples() {
        let mut first = fixture();
        // Zero-valued fills still carry entry/mitigation counts and must survive.
        let flow = &mut first.histograms.get_mut("h").unwrap().underflow_bin;
        flow.sum_weights = 0.;
        flow.sum_weights_squared = 0.;
        let mut empty = GammaLoopAccumulatorState::cleared_bundle(&first);
        empty.histograms.get_mut("h").unwrap().sample_count = 1;
        let mut encoder = Encoder::default();
        let mut bytes = vec![];
        encoder.append(&[2., -4.], &first, &mut bytes).unwrap();
        encoder.append(&[0., 0.], &empty, &mut bytes).unwrap();
        let mut decoder = Decoder::default();
        let mut actual = vec![];
        decoder
            .decode(
                header(encoder.schema().cloned()),
                &bytes,
                2,
                &mut |sample| {
                    actual.push(sample);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(actual[0].values, [2., -4.]);
        assert_eq!(actual[0].observables.as_ref(), Some(&first));
        assert_eq!(actual[1].observables.as_ref(), Some(&empty));
        actual.clear();
        decoder
            .decode(header(None), &bytes, 2, &mut |sample| {
                actual.push(sample);
                Ok(())
            })
            .unwrap();
        assert_eq!(actual.len(), 2);
        assert!(
            decoder
                .decode(
                    header(encoder.schema().cloned()),
                    &bytes,
                    2,
                    &mut |_| Ok(())
                )
                .is_err()
        );
        let mut changed = first;
        changed.histograms.get_mut("h").unwrap().title = "Changed".into();
        assert!(encoder.append(&[0., 0.], &changed, &mut vec![]).is_err());
    }

    #[test]
    fn rejects_corrupt_rows_without_panics() {
        let mut encoder = Encoder::default();
        let mut bytes = vec![];
        encoder.append(&[2., -4.], &fixture(), &mut bytes).unwrap();
        let decode = |b: &[u8]| {
            Decoder::default().decode(header(encoder.schema().cloned()), b, 1, &mut |_| Ok(()))
        };
        for size in 0..bytes.len() {
            assert!(decode(&bytes[..size]).is_err());
        }
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(decode(&extra).is_err());
        let mut nonfinite = bytes.clone();
        nonfinite[..8].copy_from_slice(&f64::NAN.to_le_bytes());
        assert!(decode(&nonfinite).is_err());
        let mut index = bytes.clone();
        index[44..48].copy_from_slice(&99u32.to_le_bytes());
        assert!(decode(&index).is_err());
        let mut duplicate = bytes.clone();
        duplicate[80..84].copy_from_slice(&0u32.to_le_bytes());
        assert!(decode(&duplicate).is_err());
        assert!(
            Decoder::default()
                .decode(header(None), &bytes, 1, &mut |_| Ok(()))
                .is_err()
        );
        Decoder::default()
            .decode(header(None), &[], 0, &mut |_| Ok(()))
            .unwrap();
    }
}
