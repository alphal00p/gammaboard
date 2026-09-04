use crate::evaluation::AccumulatorState;
use serde_json::{Map, Value as JsonValue, json};
use std::collections::BTreeSet;

/// Combine normalized observables from independent accumulator results.
/// Values add with signed coefficients and variances add in quadrature.
pub(crate) fn combine_independent_observables(
    inputs: &[(f64, &AccumulatorState)],
) -> Option<JsonValue> {
    let child_histograms = inputs
        .iter()
        .map(|(_, accumulator)| {
            let value = accumulator.to_json().ok()?;
            value
                .pointer("/bundle/histograms")
                .and_then(JsonValue::as_object)
                .cloned()
        })
        .collect::<Option<Vec<_>>>()?;
    let names = child_histograms
        .iter()
        .flat_map(|histograms| histograms.keys().cloned())
        .collect::<BTreeSet<_>>();
    if names.is_empty() {
        return None;
    }

    let mut histograms = Map::new();
    let mut omitted = Vec::new();
    for name in names {
        let histograms_with_coefficients = child_histograms
            .iter()
            .zip(inputs)
            .map(|(histograms, (coefficient, _))| {
                histograms
                    .get(&name)
                    .map(|histogram| (*coefficient, histogram))
            })
            .collect::<Option<Vec<_>>>();
        match histograms_with_coefficients
            .as_deref()
            .and_then(combine_histogram)
        {
            Some(histogram) => {
                histograms.insert(name, histogram);
            }
            None => omitted.push(name),
        }
    }
    let primary_histogram_name = ["higgs_pt", "integral"]
        .into_iter()
        .find(|name| histograms.contains_key(*name))
        .map(str::to_string)
        .or_else(|| histograms.keys().next().cloned());
    Some(json!({
        "primary_histogram_name": primary_histogram_name,
        "histograms": histograms,
        "omitted_incompatible_histograms": omitted,
        "expands_to": { "kind": "histogram", "source": "selected_row" },
        "expanded_label": "Combined Histogram",
        "actions": { "export_json": true, "export_hwu": false, "upload_bundle": true },
    }))
}

fn combine_histogram(inputs: &[(f64, &JsonValue)]) -> Option<JsonValue> {
    let (_, template) = inputs.first()?;
    let template_bins = template.get("bins")?.as_array()?;
    let layout = histogram_layout(template)?;
    if inputs
        .iter()
        .any(|(_, histogram)| histogram_layout(histogram).as_ref() != Some(&layout))
    {
        return None;
    }

    let mut bins = Vec::with_capacity(template_bins.len());
    for (index, template_bin) in template_bins.iter().enumerate() {
        let (value, error) = combine_bin(inputs.iter().map(|(coefficient, histogram)| {
            Some((
                *coefficient,
                histogram.get("sample_count")?,
                histogram.get("bins")?.as_array()?.get(index)?,
            ))
        }))?;
        let (start, stop) = histogram_bin_range(template, template_bin, index);
        let mut bin = Map::new();
        bin.insert("start".to_string(), json!(start));
        bin.insert("stop".to_string(), json!(stop));
        bin.insert("value".to_string(), json!(value));
        bin.insert("error".to_string(), json!(error));
        for key in ["label", "bin_id"] {
            if let Some(value) = template_bin.get(key) {
                bin.insert(key.to_string(), value.clone());
            }
        }
        bins.push(JsonValue::Object(bin));
    }

    let mut combined = template.as_object()?.clone();
    combined.insert("bins".to_string(), JsonValue::Array(bins));
    for key in ["underflow_bin", "overflow_bin"] {
        if template.get(key).is_none() {
            combined.remove(key);
            continue;
        }
        let (value, error) = combine_bin(inputs.iter().map(|(coefficient, histogram)| {
            Some((
                *coefficient,
                histogram.get("sample_count")?,
                histogram.get(key)?,
            ))
        }))?;
        combined.insert(key.to_string(), json!({ "value": value, "error": error }));
    }
    combined.insert(
        "sample_count".to_string(),
        json!(
            inputs
                .iter()
                .filter_map(|(_, histogram)| histogram.get("sample_count")?.as_i64())
                .sum::<i64>()
        ),
    );
    combined.remove("statistics");
    Some(JsonValue::Object(combined))
}

fn combine_bin<'a>(
    inputs: impl Iterator<Item = Option<(f64, &'a JsonValue, &'a JsonValue)>>,
) -> Option<(f64, f64)> {
    let mut value = 0.0;
    let mut variance = 0.0;
    for input in inputs {
        let (coefficient, sample_count, bin) = input?;
        let sample_count = sample_count.as_f64()?;
        let sum = bin.get("sum_weights")?.as_f64()?;
        let sum_sq = bin.get("sum_weights_squared")?.as_f64()?;
        if sample_count <= 0.0 {
            return None;
        }
        let mean = sum / sample_count;
        let error_sq = if sample_count > 1.0 {
            ((sum_sq - sum * sum / sample_count) / (sample_count * (sample_count - 1.0))).max(0.0)
        } else {
            0.0
        };
        value += coefficient * mean;
        variance += coefficient.powi(2) * error_sq;
    }
    Some((value, variance.sqrt()))
}

fn histogram_layout(histogram: &JsonValue) -> Option<JsonValue> {
    let object = histogram.as_object()?;
    let mut layout = Map::new();
    for key in [
        "kind",
        "phase",
        "value_transform",
        "x_min",
        "x_max",
        "discrete_min_bin_id",
        "discrete_ordering",
    ] {
        if let Some(value) = object.get(key) {
            layout.insert(key.to_string(), value.clone());
        }
    }
    layout.insert(
        "bins".to_string(),
        JsonValue::Array(
            object
                .get("bins")?
                .as_array()?
                .iter()
                .map(|bin| {
                    json!({
                        "x_min": bin.get("x_min"),
                        "x_max": bin.get("x_max"),
                        "bin_id": bin.get("bin_id"),
                        "label": bin.get("label"),
                    })
                })
                .collect(),
        ),
    );
    Some(JsonValue::Object(layout))
}

fn histogram_bin_range(histogram: &JsonValue, bin: &JsonValue, index: usize) -> (f64, f64) {
    if let (Some(start), Some(stop)) = (
        bin.get("x_min").and_then(JsonValue::as_f64),
        bin.get("x_max").and_then(JsonValue::as_f64),
    ) {
        return (start, stop);
    }
    let id = bin
        .get("bin_id")
        .and_then(JsonValue::as_f64)
        .or_else(|| {
            histogram
                .get("discrete_min_bin_id")
                .and_then(JsonValue::as_f64)
                .map(|minimum| minimum + index as f64)
        })
        .unwrap_or(index as f64);
    (id, id + 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combines_histogram_values_and_independent_errors() {
        let histogram = |sum: f64, sum_sq: f64| {
            json!({
                "kind": "continuous",
                "title": "observable",
                "phase": "real",
                "value_transform": "identity",
                "sample_count": 2,
                "bins": [{
                    "x_min": 0.0,
                    "x_max": 1.0,
                    "entry_count": 2,
                    "sum_weights": sum,
                    "sum_weights_squared": sum_sq
                }]
            })
        };
        let left = histogram(4.0, 10.0);
        let right = histogram(6.0, 20.0);
        let combined = combine_histogram(&[(2.0, &left), (-0.5, &right)]).expect("combined");
        let bin = &combined["bins"][0];

        assert_eq!(bin["value"], json!(2.5));
        assert!((bin["error"].as_f64().unwrap() - 4.25_f64.sqrt()).abs() < 1e-12);
        assert_eq!(combined["sample_count"], json!(4));
    }

    #[test]
    fn rejects_incompatible_histogram_layouts() {
        let left = json!({
            "kind": "continuous", "phase": "real", "sample_count": 2,
            "bins": [{"x_min": 0.0, "x_max": 1.0, "sum_weights": 1.0, "sum_weights_squared": 1.0}]
        });
        let right = json!({
            "kind": "continuous", "phase": "real", "sample_count": 2,
            "bins": [{"x_min": 0.0, "x_max": 2.0, "sum_weights": 1.0, "sum_weights_squared": 1.0}]
        });

        assert!(combine_histogram(&[(1.0, &left), (1.0, &right)]).is_none());
    }
}
