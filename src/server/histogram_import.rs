use super::{ApiError, Json, json_response};
use serde_json::{Map, Value, json};

pub(super) const MAX_IMPORT_BYTES: usize = 16 * 1024 * 1024;

pub(super) async fn import_hwu(contents: String) -> Result<Json<Value>, ApiError> {
    let payload = tokio::task::spawn_blocking(move || parse_hwu(&contents))
        .await
        .map_err(|err| ApiError::Internal(format!("histogram import failed: {err}")))?
        .map_err(ApiError::BadRequest)?;
    json_response(payload)
}

/// HwU contains estimates, not the raw moments required to restore an accumulator.
/// Return normalized comparison bins without fabricating counts or second moments.
fn parse_hwu(contents: &str) -> Result<Value, String> {
    if contents.len() > MAX_IMPORT_BYTES {
        return Err("Histogram bundles must be at most 16 MiB.".into());
    }
    let mut histograms = Map::new();
    let mut warnings = Vec::new();
    let mut columns: Vec<String> = Vec::new();
    let mut current: Option<(String, usize, Value, Vec<Value>)> = None;
    let mut total_bins = 0;
    for (index, line) in contents.trim_start_matches('\u{feff}').lines().enumerate() {
        let line = line.trim();
        let error = |message: &str| format!("HwU line {}: {message}", index + 1);
        if line.is_empty() {
            continue;
        }
        if let Some(header) = line.strip_prefix("##&") {
            if current.is_some() {
                return Err(error("column header inside histogram"));
            }
            columns = header
                .split('&')
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect();
            if columns.len() < 4 || columns[..4] != ["xmin", "xmax", "central value", "dy"] {
                return Err(error("expected columns xmin, xmax, central value, dy"));
            }
            if columns.len() > 4 && warnings.is_empty() {
                warnings.push("Imported central value and dy only; additional HwU weight columns were not imported.");
            }
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        if let Some(header) = line.strip_prefix("<histogram>") {
            if current.is_some() {
                return Err(error("previous histogram is not closed"));
            }
            if histograms.len() >= 10_000 {
                return Err(error("too many histograms"));
            }
            let (count, description) = header
                .trim()
                .split_once(char::is_whitespace)
                .ok_or_else(|| error("expected bin count and quoted title"))?;
            let count: usize = count.parse().map_err(|_| error("invalid bin count"))?;
            if count == 0 || count > 100_000 - total_bins {
                return Err(error("expected 1–100000 bins in the bundle"));
            }
            let description = description
                .trim()
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .ok_or_else(|| error("expected a quoted histogram title"))?;
            let mut parts = description.split('|');
            let title = parts.next().unwrap_or_default().trim();
            if title.is_empty() {
                return Err(error("empty histogram title"));
            }
            let mut histogram =
                json!({"title": title, "value_convention": "bin_integral", "source_format": "hwu"});
            for field in parts {
                if let Some((key, value)) = field.trim().split_once('@') {
                    match key.trim() {
                        "X_AXIS" => histogram["log_x_axis"] = json!(value.trim() == "LOG"),
                        "Y_AXIS" => histogram["log_y_axis"] = json!(value.trim() == "LOG"),
                        "TYPE" => histogram["type_description"] = json!(value.trim()),
                        _ => (),
                    }
                }
            }
            let mut name = title.to_owned();
            let mut suffix = 2;
            while histograms.contains_key(&name) {
                name = format!("{title} ({suffix})");
                suffix += 1;
            }
            current = Some((name, count, histogram, Vec::new()));
            continue;
        }
        if line == "<\\histogram>" || line == "</histogram>" {
            let (name, count, mut histogram, bins) = current
                .take()
                .ok_or_else(|| error("unexpected histogram end"))?;
            if bins.len() != count {
                return Err(error(&format!(
                    "histogram '{name}' declares {count} bins but contains {}",
                    bins.len()
                )));
            }
            histogram["bins"] = json!(bins);
            histograms.insert(name, histogram);
            total_bins += count;
            continue;
        }
        let Some((name, count, _, bins)) = current.as_mut() else {
            return Err(error("expected <histogram> or column header"));
        };
        let fields: Vec<_> = line
            .split('#')
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .collect();
        let expected_columns = if columns.is_empty() { 4 } else { columns.len() };
        if fields.len() != expected_columns {
            return Err(error(&format!(
                "histogram '{name}' expects {expected_columns} columns, got {}",
                fields.len()
            )));
        }
        let values = fields
            .iter()
            .map(|s| s.replace(['D', 'd'], "e").parse::<f64>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| error(&format!("invalid number in histogram '{name}'")))?;
        if values.iter().any(|v| !v.is_finite()) || values[0] >= values[1] || values[3] < 0.0 {
            return Err(error(&format!(
                "histogram '{name}' requires finite values, increasing edges and nonnegative errors"
            )));
        }
        if bins
            .last()
            .is_some_and(|bin| bin["stop"].as_f64().unwrap() > values[0])
        {
            return Err(error(&format!(
                "histogram '{name}' has overlapping or unordered bins"
            )));
        }
        if bins.len() >= *count {
            return Err(error(&format!("too many bins in histogram '{name}'")));
        }
        bins.push(
            json!({"start":values[0], "stop":values[1], "value":values[2], "error":values[3]}),
        );
    }
    if let Some((name, ..)) = current {
        return Err(format!("HwU histogram '{name}' is not closed"));
    }
    if histograms.is_empty() {
        return Err("HwU contains no histograms".into());
    }
    Ok(json!({"histograms":histograms, "warnings":warnings}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_metadata_duplicate_titles_and_explicit_extra_columns() {
        let block = "<histogram> 2 \"Energy |X_AXIS@LOG |Y_AXIS@LIN |TYPE@LO\"\n0 1 -2D+0 0.1 5\n1 3 4 0.2 6\n<\\histogram>\n";
        let result = parse_hwu(&format!(
            "##& xmin & xmax & central value & dy & scale variation &\n{block}{block}"
        ))
        .unwrap();
        assert_eq!(result["histograms"].as_object().unwrap().len(), 2);
        assert_eq!(result["histograms"]["Energy"]["bins"][0]["value"], -2.0);
        assert_eq!(result["histograms"]["Energy (2)"]["log_x_axis"], true);
        assert_eq!(result["histograms"]["Energy"]["type_description"], "LO");
        assert_eq!(result["warnings"].as_array().unwrap().len(), 1);
        assert!(result["histograms"]["Energy"].get("sample_count").is_none());
    }

    #[test]
    fn rejects_malformed_histograms_with_line_context() {
        for body in ["0 1 NaN 0.1", "1 0 2 0.1", "0 1 2 -1", "0 1 2 0.1 7"] {
            let error =
                parse_hwu(&format!("<histogram> 1 \"bad\"\n{body}\n</histogram>")).unwrap_err();
            assert!(error.contains("line 2") && error.contains("bad"), "{error}");
        }
        assert!(
            parse_hwu("<histogram> 2 \"short\"\n0 1 2 0.1\n</histogram>")
                .unwrap_err()
                .contains("declares 2")
        );
        assert!(
            parse_hwu("<histogram> 1 \"open\"")
                .unwrap_err()
                .contains("not closed")
        );
        assert!(parse_hwu("{}").is_err());
    }

    #[test]
    #[cfg(feature = "gammaloop")]
    fn imports_gammaloop_export_without_changing_estimates() {
        use gammalooprs::observables::{
            HistogramAccumulatorState, ObservablePhase, ObservableSnapshotBundle,
            ObservableValueTransform,
        };
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
        histogram.fill_continuous_sample(&[(0.5, 2.0)]).unwrap();
        histogram.fill_continuous_sample(&[(1.5, 4.0)]).unwrap();
        histogram.update_results();
        let snapshot = histogram.snapshot();
        let bundle = ObservableSnapshotBundle {
            histograms: [("energy".into(), snapshot.clone())].into(),
        };
        let file = tempfile::NamedTempFile::new().unwrap();
        bundle.write_hwu_file(file.path()).unwrap();
        let exported =
            super::super::histogram_export::write_hwu(&serde_json::to_value(&bundle).unwrap())
                .unwrap();
        assert_eq!(exported, std::fs::read_to_string(file.path()).unwrap());
        let result = parse_hwu(&exported).unwrap();
        for (index, bin) in snapshot.bins.iter().enumerate() {
            assert_eq!(
                result["histograms"]["Energy"]["bins"][index]["value"],
                bin.average(snapshot.sample_count)
            );
            assert_eq!(
                result["histograms"]["Energy"]["bins"][index]["error"],
                bin.error(snapshot.sample_count)
            );
        }
    }
}
