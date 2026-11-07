use super::ApiError;
use serde_json::Value;
use std::fmt::Write;

#[derive(serde::Deserialize)]
pub(super) struct CampaignExportRequest {
    run_id: i32,
    task_id: String,
    format: String,
    #[serde(default)]
    include_children: bool,
}

pub(super) async fn export_campaign(
    axum::extract::State(state): axum::extract::State<super::AppState>,
    axum::Json(request): axum::Json<CampaignExportRequest>,
) -> Result<axum::Json<Value>, ApiError> {
    let task_id = request
        .task_id
        .parse::<i64>()
        .map_err(|_| ApiError::BadRequest("invalid task id".into()))?;
    let task = super::load_run_task(&state.store, request.run_id, task_id).await?;
    let crate::core::RunTaskSpec::IntegrationCampaign { children, .. } = &task.task else {
        return Err(ApiError::BadRequest(
            "This task is not an integration campaign".into(),
        ));
    };
    let runs = crate::runners::controller_child::load_controller_children(
        &state.store,
        request.run_id,
        &task,
    )
    .await?;
    // Read each published accumulator once. Both aggregate and optional child
    // exports are constructed from these same snapshots, without persisting copies.
    let mut inputs = Vec::with_capacity(children.len());
    let mut exported_children = Vec::new();
    for child in children {
        let run = runs.get(&child.name).ok_or_else(|| {
            ApiError::BadRequest(format!("Child '{}' has not been created yet", child.name))
        })?;
        let (published, _, _) =
            crate::runners::controller_child::load_published_child_result(&state.store, run.run_id)
                .await?;
        let accumulator = published.accumulator.ok_or_else(|| {
            ApiError::BadRequest(format!(
                "Child '{}' has no published accumulator",
                child.name
            ))
        })?;
        if request.include_children {
            let bundle =
                crate::services::results::observable_payload(&accumulator).ok_or_else(|| {
                    ApiError::BadRequest(format!("Child '{}' has no observable bundle", child.name))
                })?;
            exported_children.push(serde_json::json!({"name": child.name, "coefficient": child.coefficient, "source": published.source, "bundle": bundle}));
        }
        inputs.push((child.coefficient, accumulator));
    }
    let references = inputs
        .iter()
        .map(|(coefficient, accumulator)| (*coefficient, accumulator))
        .collect::<Vec<_>>();
    let observables = crate::services::results::combine_independent_observables(&references)
        .ok_or_else(|| {
            ApiError::BadRequest("Children do not yet have usable observable bundles".into())
        })?;
    let result =
        serde_json::json!({"observables": observables, "observable_children": exported_children});
    let format = request.format.trim().to_ascii_lowercase();
    let payload = prepare_campaign_export(&result, request.include_children, &format)?;
    super::export_histogram_bundle(axum::Json(super::HistogramBundleExportRequest {
        payload,
        format,
    }))
    .await
}

fn prepare_campaign_export(
    result: &Value,
    include_children: bool,
    format: &str,
) -> Result<Value, ApiError> {
    let mut bundle = result
        .get("observables")
        .filter(|value| value.get("histograms").is_some())
        .cloned()
        .ok_or_else(|| ApiError::BadRequest("This result has no campaign observables".into()))?;
    if format == "json" {
        let histograms = bundle["histograms"]
            .as_object_mut()
            .ok_or_else(|| ApiError::BadRequest("invalid histogram bundle".into()))?;
        for (name, histogram) in histograms {
            *histogram = derived_histogram(name, histogram)?;
        }
        bundle["statistics_kind"] = serde_json::json!("derived_moments");
        bundle["estimator_note"] = serde_json::json!(
            "Independent child contributions combined with campaign coefficients. Bin moments encode the combined estimates and are not pooled raw samples; do not use them to resume sampling or merge with ordinary accumulators."
        );
    }

    if include_children {
        let children = result["observable_children"]
            .as_array()
            .filter(|children| !children.is_empty())
            .ok_or_else(|| {
                ApiError::BadRequest("No child observable bundles are available".into())
            })?;
        bundle["children"] = Value::Array(children.clone());
    }
    Ok(bundle)
}

fn derived_histogram(name: &str, original: &Value) -> Result<Value, ApiError> {
    let invalid = |reason: &str| {
        ApiError::BadRequest(format!(
            "Cannot encode aggregate histogram '{name}' as GammaLoop JSON: {reason}. HwU retains the values and errors directly."
        ))
    };
    let n = original["sample_count"]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or_else(|| invalid("missing positive sample count"))? as f64;
    let mut histogram = original.clone();
    let convert_bin = |bin: &mut Value| -> Result<(), ApiError> {
        let value = bin["value"]
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or_else(|| invalid("missing finite value"))?;
        let error = bin["error"]
            .as_f64()
            .filter(|e| e.is_finite() && *e >= 0.0)
            .ok_or_else(|| invalid("missing finite nonnegative error"))?;
        for key in ["entry_count", "mitigated_fill_count"] {
            if bin[key].as_u64().is_none() {
                return Err(invalid("raw entry counters unavailable"));
            }
        }
        let sum = n * value;
        // Use the same floating-point centering expression as GammaLoop's reader.
        let centered = sum * sum / n;
        let second = centered + n * (n - 1.0) * error * error;
        let decoded_error = if n > 1.0 {
            ((second - centered).max(0.0) / (n * (n - 1.0))).sqrt()
        } else {
            0.0
        };
        let close = |a: f64, b: f64, tolerance: f64| {
            a.is_finite()
                && b.is_finite()
                && (a == b || (a - b).abs() <= tolerance * a.abs().max(b.abs()))
        };
        if !sum.is_finite()
            || !second.is_finite()
            || !close(sum / n, value, 1e-12)
            || !close(decoded_error, error, 1e-6)
        {
            return Err(invalid(
                "derived moments cannot reproduce the uncertainty at float64 precision",
            ));
        }
        let object = bin.as_object_mut().ok_or_else(|| invalid("invalid bin"))?;
        object.insert("sum_weights".into(), serde_json::json!(sum));
        object.insert("sum_weights_squared".into(), serde_json::json!(second));
        for key in ["start", "stop", "value", "error"] {
            object.remove(key);
        }
        for key in ["x_min", "x_max", "bin_id", "label"] {
            object.entry(key).or_insert(Value::Null);
        }
        Ok(())
    };
    for bin in histogram["bins"]
        .as_array_mut()
        .ok_or_else(|| invalid("missing bins"))?
    {
        convert_bin(bin)?;
    }
    for key in ["underflow_bin", "overflow_bin"] {
        convert_bin(&mut histogram[key])?;
    }
    for key in [
        "in_range_entry_count",
        "nan_value_count",
        "mitigated_pair_count",
    ] {
        if histogram["statistics"][key].as_u64().is_none() {
            return Err(invalid("raw diagnostic counters unavailable"));
        }
    }
    #[cfg(feature = "gammaloop")]
    serde_json::from_value::<gammalooprs::observables::HistogramSnapshot>(histogram.clone())
        .map_err(|err| invalid(&format!("{err}")))?;
    Ok(histogram)
}

/// Campaign results already contain estimates. Export these directly, without
/// inventing accumulator moments or normalizing them by sample_count again.
pub(super) fn write_hwu(bundle: &Value) -> Result<String, ApiError> {
    let histograms = bundle["histograms"]
        .as_object()
        .ok_or_else(|| ApiError::BadRequest("expected a histograms object".into()))?;
    let mut contents = String::new();
    for (name, histogram) in histograms {
        let bins = histogram["bins"]
            .as_array()
            .ok_or_else(|| ApiError::BadRequest(format!("histogram '{name}' has no bins array")))?;
        if !bins.iter().any(|bin| bin.get("value").is_some()) {
            contents.push_str(&write_accumulator_hwu(name, histogram)?);
            continue;
        }
        let title = hwu_metadata(histogram["title"].as_str().unwrap_or(name));
        let description = hwu_metadata(histogram["type_description"].as_str().unwrap_or(""));
        let axis = |key: &str| {
            if histogram[key].as_bool() == Some(true) {
                "LOG"
            } else {
                "LIN"
            }
        };
        writeln!(contents, "##& xmin & xmax & central value & dy &\n").unwrap();
        writeln!(
            contents,
            "<histogram> {} \"{title} |X_AXIS@{} |Y_AXIS@{} |TYPE@{description}\"",
            bins.len(),
            axis("log_x_axis"),
            axis("log_y_axis")
        )
        .unwrap();
        for (index, bin) in bins.iter().enumerate() {
            let number = |key: &str| {
                bin[key]
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| {
                        ApiError::BadRequest(format!(
                            "histogram '{name}' bin {index} needs a finite {key} for HwU export"
                        ))
                    })
            };
            let (start, stop, value, error) = (
                number("start")?,
                number("stop")?,
                number("value")?,
                number("error")?,
            );
            if start >= stop || error < 0.0 {
                return Err(ApiError::BadRequest(format!(
                    "histogram '{name}' bin {index} needs increasing edges and a nonnegative error"
                )));
            }
            writeln!(
                contents,
                "  {start:.16e}   {stop:.16e}   {value:.16e}   {error:.16e}"
            )
            .unwrap();
        }
        writeln!(contents, "<\\histogram>").unwrap();
    }
    if let Some(children) = bundle["children"].as_array() {
        for (index, child) in children.iter().enumerate() {
            let label = hwu_metadata(child["name"].as_str().unwrap_or("child"));
            writeln!(
                contents,
                "# Child {}: {label}; coefficient {}; original unscaled observables",
                index + 1,
                child["coefficient"]
            )
            .unwrap();
            let mut child_bundle = child["bundle"].clone();
            let histograms = child_bundle["histograms"]
                .as_object_mut()
                .ok_or_else(|| ApiError::BadRequest("invalid child histogram bundle".into()))?;
            for (name, histogram) in histograms {
                let title = histogram["title"].as_str().unwrap_or(name);
                histogram["title"] = serde_json::json!(format!(
                    "Child {} ({label}) / {}: {}",
                    index + 1,
                    hwu_metadata(name),
                    hwu_metadata(title)
                ));
            }
            // Only the child's histogram map is exported; nested exports are not followed.
            contents.push_str(&write_hwu(
                &serde_json::json!({"histograms": child_bundle["histograms"]}),
            )?);
        }
    }
    Ok(contents)
}

fn hwu_metadata(value: &str) -> String {
    value.replace(['\n', '\r', '|', '"'], " ")
}

fn write_accumulator_hwu(name: &str, histogram: &Value) -> Result<String, ApiError> {
    let mut histogram = histogram.clone();
    histogram["title"] =
        serde_json::json!(hwu_metadata(histogram["title"].as_str().unwrap_or(name)));
    histogram["type_description"] = serde_json::json!(hwu_metadata(
        histogram["type_description"].as_str().unwrap_or("")
    ));
    crate::services::results::histogram_hwu(&serde_json::json!({"histograms": {name: histogram}}))
        .map_err(ApiError::BadRequest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn campaign_bundle_exports_all_estimates_without_renormalizing() {
        let bundle = json!({"histograms": {
            "a": {"title": "Energy", "sample_count": 1000, "log_x_axis": true,
                "bins": [{"start": 1.0, "stop": 2.5, "value": -2.5, "error": 0.3},
                         {"start": 2.5, "stop": 7.0, "value": 4.0, "error": 0.5}],
                "underflow_bin": {"value": 99.0, "error": 1.0}},
            "b": {"title": "Channel", "bins": [{"start": -1.0, "stop": 0.0, "bin_id": -1, "value": 7.0, "error": 0.0}]}
        }, "omitted_incompatible_histograms": ["other"]});
        let mut payload = bundle.clone();
        payload["actions"] = json!({"export": true, "upload_bundle": true});
        payload["expands_to"] = json!({"kind": "histogram", "source": "selected_row"});
        payload["expanded_label"] = json!("Combined Observable");
        for format in ["json", "hwu"] {
            let response = super::super::export_histogram_bundle(axum::Json(
                super::super::HistogramBundleExportRequest {
                    payload: payload.clone(),
                    format: format.into(),
                },
            ))
            .await
            .unwrap()
            .0;
            let contents = response["contents"].as_str().unwrap();
            if format == "json" {
                let exported: Value = serde_json::from_str(contents).unwrap();
                assert_eq!(exported, bundle);
            } else {
                let exported = super::super::histogram_import::import_hwu(contents.into())
                    .await
                    .unwrap()
                    .0;
                assert_eq!(exported["histograms"].as_object().unwrap().len(), 2);
                assert_eq!(
                    exported["histograms"]["Energy"]["bins"],
                    bundle["histograms"]["a"]["bins"]
                );
                assert_eq!(exported["histograms"]["Channel"]["bins"][0]["value"], 7.0);
                assert_eq!(exported["histograms"]["Energy"]["log_x_axis"], true);
            }
        }
    }

    #[test]
    fn refuses_missing_uncertainties_instead_of_exporting_zero() {
        assert!(write_hwu(&json!({"histograms": {"x": {"bins": [{"start": 0, "stop": 1, "value": 2, "error": null}]}}})).is_err());
    }

    #[tokio::test]
    async fn exports_stratified_estimates_and_preserves_unavailable_errors() {
        let histogram = json!({"title": "Energy", "sample_count": 10,
            "bins": [{"x_min": 0.0, "x_max": 1.0, "sum_weights": 100.0,
                "sum_weights_squared": 2000.0, "estimate": {"mean": 3.0, "error": 0.5}}]});
        let mut bundle = json!({"histograms": {"energy": histogram}});
        let imported = super::super::histogram_import::import_hwu(write_hwu(&bundle).unwrap())
            .await
            .unwrap()
            .0;
        assert_eq!(imported["histograms"]["Energy"]["bins"][0]["value"], 3.0);
        assert_eq!(imported["histograms"]["Energy"]["bins"][0]["error"], 0.5);
        bundle["histograms"]["energy"]["bins"][0]["estimate"]["error"] = Value::Null;
        assert!(write_hwu(&bundle).is_err());
        bundle["histograms"]["energy"]["bins"][0]
            .as_object_mut()
            .unwrap()
            .remove("estimate");
        bundle["histograms"]["energy"]["sample_count"] = json!(1);
        assert!(write_hwu(&bundle).is_err());
    }

    #[cfg(feature = "gammaloop")]
    fn example_campaign() -> Value {
        use crate::evaluation::{AccumulatorState, accumulator::GammaLoopAccumulatorState};
        use gammalooprs::observables::{
            HistogramAccumulatorState, ObservablePhase, ObservableValueTransform,
        };
        let make = |samples: &[f64]| {
            let mut histogram = HistogramAccumulatorState::continuous(
                "Energy".into(),
                "LO".into(),
                ObservablePhase::Real,
                ObservableValueTransform::Identity,
                0.0,
                2.0,
                false,
                false,
                2,
            );
            for weight in samples {
                histogram
                    .fill_continuous_sample(&[
                        (0.5, *weight),
                        (1.5, -*weight),
                        (-1.0, *weight * 2.0),
                        (3.0, *weight * 3.0),
                    ])
                    .unwrap();
            }
            histogram.update_results();
            let mut state = GammaLoopAccumulatorState::default();
            state
                .bundle
                .histograms
                .insert("energy".into(), histogram.snapshot());
            AccumulatorState::Gammaloop(state)
        };
        let mut left = make(&[1.0, 3.0]);
        let mut right = make(&[2.0, 4.0, 6.0]);
        // One observable is absent and one has incompatible geometry. Neither
        // belongs in the aggregate, but the original child bundles retain them.
        if let AccumulatorState::Gammaloop(state) = &mut left {
            let original = state.bundle.histograms["energy"].clone();
            state
                .bundle
                .histograms
                .insert("only_left".into(), original.clone());
            state
                .bundle
                .histograms
                .insert("different_bins".into(), original);
        }
        if let AccumulatorState::Gammaloop(state) = &mut right {
            let mut incompatible = state.bundle.histograms["energy"].clone();
            incompatible.bins[0].x_max = Some(0.75);
            state
                .bundle
                .histograms
                .insert("different_bins".into(), incompatible);
        }
        let observables = crate::services::results::combine_independent_observables(&[
            (2.0, &left),
            (-0.5, &right),
        ])
        .unwrap();
        json!({"observables": observables, "observable_children": [
            {"name": "left", "coefficient": 2.0, "source": {"run_id": 1, "sample_count": 2}, "bundle": crate::services::results::observable_payload(&left).unwrap()},
            {"name": "right", "coefficient": -0.5, "source": {"run_id": 2, "sample_count": 3}, "bundle": crate::services::results::observable_payload(&right).unwrap()}
        ]})
    }

    #[tokio::test]
    #[cfg(feature = "gammaloop")]
    async fn derived_campaign_json_roundtrips_through_gammaloop_and_children_are_optional() {
        use gammalooprs::observables::ObservableSnapshotBundle;
        let result = example_campaign();
        let aggregate = prepare_campaign_export(&result, false, "json").unwrap();
        assert!(aggregate.get("children").is_none());
        assert_eq!(aggregate["statistics_kind"], "derived_moments");
        assert_eq!(
            aggregate["omitted_incompatible_histograms"],
            json!(["different_bins", "only_left"])
        );
        let decoded: ObservableSnapshotBundle = serde_json::from_value(aggregate.clone()).unwrap();
        assert_eq!(decoded.histograms.len(), 1);
        let histogram = &decoded.histograms["energy"];
        assert_eq!(histogram.sample_count, 5);
        assert_eq!(histogram.bins[0].entry_count, 5);
        assert_eq!(histogram.statistics.in_range_entry_count, 10);
        assert!((histogram.bins[0].average(5) - 2.0).abs() < 1e-12);
        assert!((histogram.bins[0].error(5).powi(2) - (4.0 + 1.0 / 3.0)).abs() < 1e-12);
        for (bin, normalized) in histogram.bins.iter().zip(
            result["observables"]["histograms"]["energy"]["bins"]
                .as_array()
                .unwrap(),
        ) {
            assert!((bin.average(5) - normalized["value"].as_f64().unwrap()).abs() < 1e-12);
            assert!((bin.error(5) - normalized["error"].as_f64().unwrap()).abs() < 1e-12);
        }
        assert_eq!(histogram.underflow_bin.entry_count, 5);
        assert_eq!(histogram.overflow_bin.entry_count, 5);
        assert!((histogram.underflow_bin.average(5) - 4.0).abs() < 1e-12);
        assert!((histogram.overflow_bin.average(5) - 6.0).abs() < 1e-12);
        let full = prepare_campaign_export(&result, true, "json").unwrap();
        assert_eq!(full["histograms"], aggregate["histograms"]);
        assert_eq!(full["children"], result["observable_children"]);
        for include in [false, true] {
            let hwu =
                write_hwu(&prepare_campaign_export(&result, include, "hwu").unwrap()).unwrap();
            let imported = super::super::histogram_import::import_hwu(hwu)
                .await
                .unwrap()
                .0;
            assert_eq!(imported["histograms"]["Energy"]["bins"][0]["value"], 2.0);
            assert_eq!(
                imported["histograms"].as_object().unwrap().len(),
                if include { 6 } else { 1 }
            );
            if include {
                assert_eq!(
                    imported["histograms"]["Child 1 (left) / energy: Energy"]["bins"][0]["value"],
                    2.0
                );
                assert_eq!(
                    imported["histograms"]["Child 2 (right) / energy: Energy"]["bins"][0]["value"],
                    4.0
                );
            }
        }
    }

    #[test]
    #[cfg(feature = "gammaloop")]
    fn derived_export_rejects_lost_errors_and_overflow_but_hwu_still_works() {
        for (value, error) in [(1.0, 1e-100), (1e308, 1.0)] {
            let mut result = example_campaign();
            let bin = &mut result["observables"]["histograms"]["energy"]["bins"][0];
            bin["value"] = json!(value);
            bin["error"] = json!(error);
            assert!(prepare_campaign_export(&result, false, "json").is_err());
            assert!(write_hwu(&prepare_campaign_export(&result, false, "hwu").unwrap()).is_ok());
        }
        let mut result = example_campaign();
        result["observables"]["histograms"]["energy"]["bins"][0]["value"] = json!(0.0);
        assert!(prepare_campaign_export(&result, false, "json").is_ok());
    }

    #[test]
    #[cfg(feature = "gammaloop")]
    fn derived_discrete_bins_and_small_sample_counts_preserve_their_meaning() {
        use gammalooprs::observables::{HistogramSnapshot, HistogramSnapshotKind};
        let mut histogram = example_campaign()["observables"]["histograms"]["energy"].clone();
        histogram["kind"] = json!("discrete");
        histogram["x_min"] = Value::Null;
        histogram["x_max"] = Value::Null;
        histogram["discrete_min_bin_id"] = json!(-2);
        for (index, bin) in histogram["bins"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            bin["x_min"] = Value::Null;
            bin["x_max"] = Value::Null;
            bin["bin_id"] = json!(index as i64 - 2);
            bin["label"] = json!(format!("channel {index}"));
        }
        let derived = derived_histogram("discrete", &histogram).unwrap();
        let decoded: HistogramSnapshot = serde_json::from_value(derived).unwrap();
        assert_eq!(decoded.kind, HistogramSnapshotKind::Discrete);
        assert_eq!(decoded.bins[0].bin_id, Some(-2));
        assert_eq!(decoded.bins[1].label.as_deref(), Some("channel 1"));
        assert_eq!(decoded.bins[0].average(5), 2.0);
        histogram["sample_count"] = json!(0);
        assert!(derived_histogram("empty", &histogram).is_err());
        histogram["sample_count"] = json!(1);
        assert!(derived_histogram("one", &histogram).is_err());
        for bin in histogram["bins"].as_array_mut().unwrap() {
            bin["error"] = json!(0.0);
        }
        histogram["underflow_bin"]["error"] = json!(0.0);
        histogram["overflow_bin"]["error"] = json!(0.0);
        let decoded: HistogramSnapshot =
            serde_json::from_value(derived_histogram("one", &histogram).unwrap()).unwrap();
        assert_eq!(decoded.bins[0].average(1), 2.0);
        assert_eq!(decoded.bins[0].error(1), 0.0);
    }
}
