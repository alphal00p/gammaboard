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
            observable_payload(accumulator)?
                .get("histograms")
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
        "estimator": "independent_sum",
        "statistics_kind": "independent_child_estimates",
        "estimator_note": "Independent child observable estimates are summed with the campaign coefficients; errors are combined in quadrature.",
        "primary_histogram_name": primary_histogram_name,
        "histograms": histograms,
        "omitted_incompatible_histograms": omitted,
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
        for key in ["label", "bin_id", "x_min", "x_max"] {
            if let Some(value) = template_bin.get(key) {
                bin.insert(key.to_string(), value.clone());
            }
        }
        add_bin_counts(
            &mut bin,
            inputs
                .iter()
                .filter_map(|(_, histogram)| histogram.get("bins")?.get(index)),
        )?;
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
        let mut bin = template.get(key)?.as_object()?.clone();
        bin.remove("sum_weights");
        bin.remove("sum_weights_squared");
        bin.remove("estimate");
        bin.remove("pooled");
        bin.insert("value".into(), json!(value));
        bin.insert("error".into(), json!(error));
        add_bin_counts(
            &mut bin,
            inputs
                .iter()
                .filter_map(|(_, histogram)| histogram.get(key)),
        )?;
        combined.insert(key.to_string(), JsonValue::Object(bin));
    }
    combined.insert(
        "sample_count".to_string(),
        json!(inputs.iter().try_fold(0u64, |total, (_, histogram)| {
            total.checked_add(histogram.get("sample_count")?.as_u64()?)
        })?),
    );
    if inputs
        .iter()
        .all(|(_, histogram)| histogram.get("statistics").is_some())
    {
        let mut statistics = Map::new();
        for key in [
            "in_range_entry_count",
            "nan_value_count",
            "mitigated_pair_count",
        ] {
            statistics.insert(
                key.into(),
                json!(inputs.iter().try_fold(0u64, |total, (_, histogram)| {
                    total.checked_add(histogram.get("statistics")?.get(key)?.as_u64()?)
                })?),
            );
        }
        combined.insert("statistics".into(), JsonValue::Object(statistics));
    } else {
        combined.remove("statistics");
    }
    combined.remove("strata");
    combined.remove("stratum_sample_counts");
    Some(JsonValue::Object(combined))
}

fn add_bin_counts<'a>(
    target: &mut Map<String, JsonValue>,
    bins: impl Iterator<Item = &'a JsonValue>,
) -> Option<()> {
    let bins = bins.collect::<Vec<_>>();
    for key in ["entry_count", "mitigated_fill_count"] {
        // Older/minimal snapshots can lack counters; do not invent them.
        if bins.iter().all(|bin| bin.get(key).is_some()) {
            target.insert(
                key.into(),
                json!(bins.iter().try_fold(0u64, |total, bin| {
                    total.checked_add(bin.get(key)?.as_u64()?)
                })?),
            );
        } else {
            target.remove(key);
        }
    }
    Some(())
}

fn combine_bin<'a>(
    inputs: impl Iterator<Item = Option<(f64, &'a JsonValue, &'a JsonValue)>>,
) -> Option<(f64, Option<f64>)> {
    let mut value = 0.0;
    let mut error = Some(0.0_f64);
    for input in inputs {
        let (coefficient, sample_count, bin) = input?;
        let (mean, uncertainty) = bin_estimate(bin, sample_count.as_f64()?)?;
        value += coefficient * mean;
        error = error
            .zip(uncertainty)
            .map(|(sum, e)| sum.hypot(coefficient * e));
    }
    Some((value, error))
}

fn bin_estimate(bin: &JsonValue, count: f64) -> Option<(f64, Option<f64>)> {
    if let Some(estimate) = bin.get("estimate") {
        return Some((
            estimate.get("mean")?.as_f64()?,
            estimate.get("error").and_then(JsonValue::as_f64),
        ));
    }
    if count <= 0. {
        return None;
    }
    let sum = bin.get("sum_weights")?.as_f64()?;
    let sum_sq = bin.get("sum_weights_squared")?.as_f64()?;
    Some((
        sum / count,
        (count > 1.).then(|| {
            ((sum_sq - sum * sum / count) / (count * (count - 1.)))
                .max(0.)
                .sqrt()
        }),
    ))
}

/// Presentation estimates are separate from raw, mergeable checkpoint statistics.
#[cfg(feature = "gammaloop")]
pub(crate) fn observable_payload(accumulator: &AccumulatorState) -> Option<JsonValue> {
    let pooled = accumulator.gammaloop()?;
    let mut payload = serde_json::to_value(&pooled.bundle).ok()?;
    payload["estimator"] = json!("pooled");
    payload["estimator_note"] = json!(
        "Pooled means and sample-variance errors; every input draw counts, including empty fills."
    );
    if let AccumulatorState::Stratified(state) = accumulator {
        let estimates =
            gammalooprs::observables::ObservableSnapshotBundle::sum_independent_estimates(
                &state.histograms,
            )
            .ok()?;
        payload["estimator"] = json!("stratified");
        payload["estimator_note"] = json!(
            "Stratified bin means and errors; raw sums and pooled estimates retain the pooled reference. Bin errors do not include covariance between bins."
        );
        payload["stratum_prefixes"] = json!(state.plan.partition.prefixes().collect::<Vec<_>>());
        let probabilities = state.plan.probabilities.as_ref()?;
        payload["strata"] = json!(
            state
                .plan
                .partition
                .prefixes()
                .zip(&state.strata)
                .zip(probabilities)
                .map(|((prefix, moments), probability)| json!({
                    "prefix": prefix,
                    "probability": probability,
                    "sample_count": moments.attempts,
                    "matches": moments.matches,
                }))
                .collect::<Vec<_>>()
        );
        for (name, estimate) in estimates.histograms {
            let reference = pooled.bundle.histograms.get(&name)?;
            let estimate_layout = state.histograms.first()?.histograms.get(&name)?;
            let indices = histogram_bin_indices(reference, estimate_layout)?;
            let histogram = &mut payload["histograms"][&name];
            histogram["stratum_sample_counts"] = json!(estimate.sample_counts);
            let count = histogram["sample_count"].as_f64()?;
            let attach =
                |bin: &mut JsonValue,
                 mut estimate: gammalooprs::observables::IndependentEstimate| {
                    if !state.uncertainty_ready() {
                        estimate.error = None;
                    }
                    bin["pooled"] = bin_estimate(bin, count)
                        .map(|(mean, error)| json!({"mean":mean,"error":error}))
                        .unwrap_or(JsonValue::Null);
                    bin["estimate"] = json!(estimate);
                };
            for (bin, index) in histogram["bins"].as_array_mut()?.iter_mut().zip(indices) {
                attach(bin, *estimate.bins.get(index)?);
            }
            attach(&mut histogram["underflow_bin"], estimate.underflow);
            attach(&mut histogram["overflow_bin"], estimate.overflow);
            // Only estimates vary by stratum; coordinates and labels are shared.
            // The pooled contribution uses the total count, including zeros from
            // draws assigned to other strata, and removes the conditional p_k.
            histogram["strata"] = json!(
                state
                    .histograms
                    .iter()
                    .zip(&state.strata)
                    .zip(probabilities)
                    .map(|((bundle, moments), &probability)| {
                        stratum_histogram_payload(
                            reference,
                            bundle.histograms.get(&name)?,
                            probability,
                            moments.uncertainty_ready(),
                        )
                    })
                    .collect::<Option<Vec<_>>>()?
            );
        }
    }
    Some(payload)
}

/// Align conditional estimates to the shared bin layout. A missing bin is an
/// incompatible payload, never an unrelated positional substitute.
#[cfg(feature = "gammaloop")]
fn stratum_histogram_payload(
    reference: &gammalooprs::observables::HistogramSnapshot,
    histogram: &gammalooprs::observables::HistogramSnapshot,
    probability: f64,
    uncertainty_ready: bool,
) -> Option<JsonValue> {
    let paired = |bin: &gammalooprs::observables::HistogramBinSnapshot| {
        json!({
            "estimate": {
                "mean": (histogram.sample_count > 0).then(|| bin.average(histogram.sample_count)),
                "error": uncertainty_ready.then(|| bin.error(histogram.sample_count)),
            },
            "pooled": {
                "mean": (reference.sample_count > 0).then(|| bin.average(reference.sample_count) / probability),
                "error": (reference.sample_count > 1).then(|| bin.error(reference.sample_count) / probability),
            },
        })
    };
    let bins = histogram_bin_indices(reference, histogram)?
        .into_iter()
        .map(|index| paired(&histogram.bins[index]))
        .collect::<Vec<_>>();
    Some(json!({
        "bins": bins,
        "underflow_bin": paired(&histogram.underflow_bin),
        "overflow_bin": paired(&histogram.overflow_bin),
    }))
}

#[cfg(feature = "gammaloop")]
fn histogram_bin_indices(
    reference: &gammalooprs::observables::HistogramSnapshot,
    source: &gammalooprs::observables::HistogramSnapshot,
) -> Option<Vec<usize>> {
    let by_id = source
        .bins
        .iter()
        .enumerate()
        .filter_map(|(index, bin)| bin.bin_id.map(|id| (id, index)))
        .collect::<std::collections::BTreeMap<_, _>>();
    reference
        .bins
        .iter()
        .enumerate()
        .map(|(index, bin)| match bin.bin_id {
            Some(id) => by_id.get(&id).copied(),
            None => source.bins.get(index).map(|_| index),
        })
        .collect()
}

#[cfg(not(feature = "gammaloop"))]
pub(crate) fn observable_payload(_accumulator: &AccumulatorState) -> Option<JsonValue> {
    None
}

/// HwU contains final values and errors, never fictitious raw moments. Unknown
/// errors cannot be represented in this format; JSON retains partial estimates.
pub(crate) fn histogram_hwu(payload: &JsonValue) -> Result<String, String> {
    use std::fmt::Write;
    let mut output = String::new();
    for histogram in payload
        .get("histograms")
        .and_then(JsonValue::as_object)
        .ok_or("missing histograms")?
        .values()
    {
        let bins = histogram
            .get("bins")
            .and_then(JsonValue::as_array)
            .ok_or("missing bins")?;
        let count = histogram
            .get("sample_count")
            .and_then(JsonValue::as_f64)
            .unwrap_or(0.);
        let axis = |key| {
            if histogram
                .get(key)
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
            {
                "LOG"
            } else {
                "LIN"
            }
        };
        writeln!(output, "##& xmin & xmax & central value & dy &\n").unwrap();
        writeln!(
            output,
            "<histogram> {} \"{} |X_AXIS@{} |Y_AXIS@{} |TYPE@{}\"",
            bins.len(),
            histogram["title"].as_str().unwrap_or(""),
            axis("log_x_axis"),
            axis("log_y_axis"),
            histogram["type_description"].as_str().unwrap_or("HwU")
        )
        .unwrap();
        for (i, bin) in bins.iter().enumerate() {
            let (mean, error) = if bin.get("value").is_some() {
                (
                    bin["value"].as_f64().ok_or("unavailable bin mean")?,
                    bin["error"].as_f64(),
                )
            } else {
                bin_estimate(bin, count).ok_or("unavailable bin mean")?
            };
            let error = error.ok_or(
                "HwU export requires available bin errors; use JSON for partial estimates",
            )?;
            let (start, stop) = histogram_bin_range(histogram, bin, i);
            writeln!(
                output,
                "  {start:.8e}   {stop:.8e}   {mean:.8e}   {error:.8e}"
            )
            .unwrap();
        }
        writeln!(output, "<\\histogram>").unwrap();
    }
    Ok(output)
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

    #[cfg(feature = "gammaloop")]
    #[test]
    fn stratified_histogram_views_follow_bin_ids_and_reject_missing_bins() {
        use crate::core::{AccumulatorConfig, StratificationConfig, StratificationPlan};
        use crate::evaluation::accumulator::StratifiedAccumulatorState;
        use crate::{Domain, Point};
        use gammalooprs::observables::{
            DiscreteBinOrdering, HistogramAccumulatorState, ObservablePhase,
            ObservableSnapshotBundle,
        };
        let plan = StratificationPlan {
            proposal_id: "bin-order".into(),
            partition: StratificationConfig::Voronoi {
                domain: Domain::strata(vec![2]),
                centers: [(vec![0], vec![-1., 0.]), (vec![1], vec![1., 0.])].into(),
                scales: None,
            },
            probabilities: Some(vec![0.5, 0.5]),
            components: vec!["real".into(), "imag".into()],
        };
        let mut state =
            StratifiedAccumulatorState::new(plan, AccumulatorConfig::Gammaloop).unwrap();
        for (label, bins) in [[2., 10.], [7., 3.]].into_iter().enumerate() {
            let mut histogram = HistogramAccumulatorState::discrete(
                "bins".into(),
                "HwU".into(),
                ObservablePhase::Real,
                10,
                11,
                DiscreteBinOrdering::AscendingBinId,
                false,
                vec![],
            )
            .unwrap();
            for _ in 0..32 {
                histogram
                    .fill_discrete_sample(&[(10, bins[0]), (11, bins[1])])
                    .unwrap();
                let values = [if label == 0 { -1. } else { 1. }, 0.];
                state.push(label, &values, 2., true).unwrap();
                state
                    .pooled
                    .as_mut()
                    .unwrap()
                    .estimate
                    .ingest_vector(&values, &Point::new(vec![0.5], vec![], 2.))
                    .unwrap();
            }
            let mut bundle = ObservableSnapshotBundle {
                histograms: [("h".into(), histogram.snapshot())].into(),
            };
            let pooled = &mut state.pooled.as_mut().unwrap().bundle;
            if label == 0 {
                *pooled = bundle.clone();
            } else {
                pooled.merge_in_place(&bundle).unwrap();
            }
            if label == 0 {
                bundle.histograms.get_mut("h").unwrap().bins.reverse();
            }
            state.histograms.push(bundle);
        }
        state
            .pooled
            .as_mut()
            .unwrap()
            .bundle
            .histograms
            .get_mut("h")
            .unwrap()
            .rescale(2.);
        state.validate().unwrap();
        let reference = &state.pooled.as_ref().unwrap().bundle.histograms["h"];
        let mut missing = state.histograms[0].histograms["h"].clone();
        missing.bins.pop();
        assert!(stratum_histogram_payload(reference, &missing, 0.5, true).is_none());
        let payload = observable_payload(&AccumulatorState::Stratified(Box::new(state))).unwrap();
        let histogram = &payload["histograms"]["h"];
        for (index, mean) in [9., 13.].into_iter().enumerate() {
            assert_eq!(histogram["bins"][index]["bin_id"], json!(10 + index));
            assert_eq!(histogram["bins"][index]["estimate"]["mean"], json!(mean));
            assert_eq!(histogram["bins"][index]["pooled"]["mean"], json!(mean));
        }
        assert_eq!(
            histogram["strata"][0]["bins"][0]["estimate"]["mean"],
            json!(2.)
        );
        assert_eq!(
            histogram["strata"][1]["bins"][1]["estimate"]["mean"],
            json!(3.)
        );
    }

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
