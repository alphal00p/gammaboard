import { asArray } from "../../utils/collections";
import { formatCompactNumber, formatScientific } from "../../utils/formatters";
import { FULL_ZOOM, isObject, normalizeZoomRange, readYZoomFromPanelValue, readZoomFromPanelValue } from "./panelView";

export const HISTOGRAM_SORT_CANONICAL = "canonical";
export const HISTOGRAM_SORT_BY_VALUE = "by_value";
export const HISTOGRAM_SORT_BY_ABS_VALUE = "by_abs_value";
export const HISTOGRAM_MODE_PDF = "pdf";
export const HISTOGRAM_MODE_CDF = "cdf";
export const HISTOGRAM_POSITIVE_COLOR = "#0a9396";
export const HISTOGRAM_NEGATIVE_COLOR = "#ca6702";
export const HISTOGRAM_ZERO_COLOR = "#6b7280";

const SIGNED_LOG_EPSILON = Number.EPSILON;

export const fitDomain = (values) => {
  const finiteValues = values.filter((value) => Number.isFinite(value));
  if (finiteValues.length === 0) return ["auto", "auto"];
  const min = Math.min(...finiteValues);
  const max = Math.max(...finiteValues);
  if (min === max) {
    const padding = Math.abs(min) > 0 ? Math.abs(min) * 0.1 : 1;
    return [min - padding, max + padding];
  }
  const padding = (max - min) * 0.08;
  return [min - padding, max + padding];
};

export const fitXDomain = (values) => {
  const finiteValues = values.filter((value) => Number.isFinite(value));
  if (finiteValues.length === 0) return ["auto", "auto"];
  const min = Math.min(...finiteValues);
  const max = Math.max(...finiteValues);
  if (min === max) {
    const padding = Math.abs(min) > 0 ? Math.abs(min) * 0.1 : 1;
    return [min - padding, max + padding];
  }
  return [min, max];
};

export const fitHistogramXDomain = (bins) => {
  const edges = bins.flatMap((bin) => [bin.start, bin.stop]).filter((value) => Number.isFinite(value));
  return fitXDomain(edges);
};

export const signedLog10 = (value) => {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return Number.NaN;
  return Math.log10(Math.max(Math.abs(numeric), SIGNED_LOG_EPSILON));
};

export const formatSignedLogAxisValue = (value) => {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return "n/a";
  return `10^${formatCompactNumber(numeric, 2)}`;
};

export const histogramSignColorFromRaw = (rawValue) => {
  const numeric = Number(rawValue);
  if (!Number.isFinite(numeric)) return HISTOGRAM_ZERO_COLOR;
  if (numeric < 0) return HISTOGRAM_NEGATIVE_COLOR;
  if (numeric > 0) return HISTOGRAM_POSITIVE_COLOR;
  return HISTOGRAM_ZERO_COLOR;
};

export const readHistogramBundleSelectedValue = (value) => {
  if (typeof value === "string") return value;
  if (isObject(value) && typeof value.selected_histogram === "string") return value.selected_histogram;
  return null;
};

export const readHistogramBundleView = (value) => {
  if (!isObject(value)) return {};
  return isObject(value.histogram_view) ? value.histogram_view : {};
};

export const readHistogramZoomFromPanelValue = (value, fallback = FULL_ZOOM) => {
  const view = readHistogramBundleView(value);
  return normalizeZoomRange(view.zoom) || readZoomFromPanelValue(value, fallback);
};

export const readHistogramYZoomFromPanelValue = (value, fallback = FULL_ZOOM) => {
  const view = readHistogramBundleView(value);
  return normalizeZoomRange(view.y_zoom) || readYZoomFromPanelValue(value, fallback);
};

export const readHistogramScaleFromPanelValue = (value, axis, fallback = "linear") => {
  const view = readHistogramBundleView(value);
  const scale = view?.[`${axis}_scale`];
  if (scale === "log") return "log";
  if (scale === "linear") return "linear";
  return fallback === "log" ? "log" : "linear";
};

export const normalizeHistogramMode = (value) => (value === HISTOGRAM_MODE_CDF ? HISTOGRAM_MODE_CDF : HISTOGRAM_MODE_PDF);

export const writeHistogramBundlePanelValue = (
  current,
  {
    selectedHistogram = undefined,
    zoom = undefined,
    yZoom = undefined,
    xScale = undefined,
    yScale = undefined,
    showRelativeError = undefined,
    showRatio = undefined,
    showPdfComparison = undefined,
    histogramMode = undefined,
    sortModeByHistogram = undefined,
  } = {},
) => {
  const next = isObject(current) ? { ...current } : {};
  const nextView = isObject(next.histogram_view) ? { ...next.histogram_view } : {};
  if (selectedHistogram !== undefined) next.selected_histogram = selectedHistogram;
  if (zoom !== undefined) nextView.zoom = normalizeZoomRange(zoom) || FULL_ZOOM;
  if (yZoom !== undefined) nextView.y_zoom = normalizeZoomRange(yZoom) || FULL_ZOOM;
  if (xScale !== undefined) nextView.x_scale = xScale === "log" ? "log" : "linear";
  if (yScale !== undefined) nextView.y_scale = yScale === "log" ? "log" : "linear";
  if (showRelativeError !== undefined) nextView.show_relative_error = Boolean(showRelativeError);
  if (showRatio !== undefined) nextView.show_ratio = Boolean(showRatio);
  if (showPdfComparison !== undefined) nextView.show_pdf_comparison = Boolean(showPdfComparison);
  if (histogramMode !== undefined) nextView.display_mode = normalizeHistogramMode(histogramMode);
  if (isObject(sortModeByHistogram)) {
    const normalizedSortModes = Object.fromEntries(
      Object.entries(sortModeByHistogram)
        .filter(([name]) => typeof name === "string")
        .map(([name, value]) => [name, normalizeHistogramSortMode(value)]),
    );
    nextView.sort_mode_by_histogram = {
      ...(isObject(nextView.sort_mode_by_histogram) ? nextView.sort_mode_by_histogram : {}),
      ...normalizedSortModes,
    };
  }
  if (Object.keys(nextView).length > 0) next.histogram_view = nextView;
  return next;
};

const computeGammaLoopBinAverage = (bin, sampleCount) => {
  const n = Number(sampleCount);
  const sumWeights = Number(bin?.sum_weights);
  if (!Number.isFinite(n) || n <= 0 || !Number.isFinite(sumWeights)) return 0;
  return sumWeights / n;
};

const computeGammaLoopBinError = (bin, sampleCount) => {
  const n = Number(sampleCount);
  const sumWeights = Number(bin?.sum_weights);
  const sumWeightsSquared = Number(bin?.sum_weights_squared);
  if (!Number.isFinite(n) || n <= 1 || !Number.isFinite(sumWeights) || !Number.isFinite(sumWeightsSquared)) {
    return 0;
  }
  const varianceNumerator = sumWeightsSquared - (sumWeights * sumWeights) / n;
  if (!Number.isFinite(varianceNumerator) || varianceNumerator <= 0) return 0;
  return Math.sqrt(varianceNumerator / (n * (n - 1)));
};

export const normalizeGammaLoopHistogramBins = (histogram) => {
  const bins = asArray(histogram?.bins);
  const sampleCount = Number(histogram?.sample_count);
  const isDiscrete = bins.some((bin) => bin && (bin.bin_id != null || bin.label != null));
  return bins.map((bin, index) => {
    const binId = Number(bin?.bin_id);
    const start = isDiscrete ? (Number.isFinite(binId) ? binId : index) : Number(bin?.x_min);
    const stop = isDiscrete ? (Number.isFinite(binId) ? binId + 1 : index + 1) : Number(bin?.x_max);
    return {
      start,
      stop,
      value: computeGammaLoopBinAverage(bin, sampleCount),
      error: computeGammaLoopBinError(bin, sampleCount),
      label: bin?.label ?? null,
      bin_id: bin?.bin_id ?? null,
      entry_count: bin?.entry_count ?? null,
      sum_weights: bin?.sum_weights ?? null,
      sum_weights_squared: bin?.sum_weights_squared ?? null,
      mitigated_fill_count: bin?.mitigated_fill_count ?? null,
    };
  });
};

export const buildHistogramData = (bins) =>
  asArray(bins)
    .map((bin) => {
      const start = Number(bin?.start);
      const stop = Number(bin?.stop);
      const value = Number(bin?.value);
      const error = Number(bin?.error);
      const x = Number.isFinite(start) && Number.isFinite(stop) ? (start + stop) / 2 : Number.NaN;
      return {
        ...bin,
        start,
        stop,
        x,
        value,
        error: Number.isFinite(error) ? error : 0,
        rangeLabel:
          Number.isFinite(start) && Number.isFinite(stop)
            ? `${formatScientific(start, 4)} → ${formatScientific(stop, 4)}`
            : "n/a",
      };
    })
    .filter((bin) => Number.isFinite(bin.value) && Number.isFinite(bin.x));

export const buildCdfBins = (bins) => {
  let cumulativeValue = 0;
  let cumulativeVariance = 0;
  return asArray(bins).map((bin) => {
    const value = Number(bin?.value);
    const error = Math.abs(Number(bin?.error));
    if (Number.isFinite(value)) cumulativeValue += value;
    if (Number.isFinite(error)) cumulativeVariance += error * error;
    return {
      ...bin,
      value: cumulativeValue,
      error: Math.sqrt(cumulativeVariance),
    };
  });
};

const sampleContinuousHistogramAtX = (bins, x) => {
  const numericX = Number(x);
  if (!Number.isFinite(numericX)) return null;
  const epsilon = 1e-12;
  for (const bin of asArray(bins)) {
    const start = Number(bin?.start);
    const stop = Number(bin?.stop);
    if (!Number.isFinite(start) || !Number.isFinite(stop)) continue;
    if (numericX + epsilon < start) continue;
    if (numericX - epsilon > stop) continue;
    return bin;
  }
  return null;
};

export const projectOverlayHistogramToReferenceBins = (referenceBins, overlayBins, yScale, xScale) => {
  const referenceCanonical = buildHistogramData(referenceBins);
  const overlayCanonical = buildHistogramData(overlayBins);
  const projectedBins = referenceCanonical
    .map((referenceBin) => {
      const matched = sampleContinuousHistogramAtX(overlayCanonical, referenceBin.x);
      if (!matched) return null;
      return {
        start: referenceBin.start,
        stop: referenceBin.stop,
        value: Number(matched.value),
        error: Number.isFinite(Number(matched.error)) ? Math.abs(Number(matched.error)) : 0,
      };
    })
    .filter((bin) => Number.isFinite(bin?.start) && Number.isFinite(bin?.stop) && Number.isFinite(bin?.value));

  const valueStep = buildHistogramRenderData(projectedBins, yScale)
    .map((point) => [Number(point?.x), Number(point?.y)])
    .filter(([x]) => Number.isFinite(x) && (xScale !== "log" || x > 0));
  const relativeStep = buildRelativeErrorStepData(projectedBins)
    .map((point) => [Number(point?.x), Number(point?.relative_error)])
    .filter(([x]) => Number.isFinite(x) && (xScale !== "log" || x > 0));
  const absError = buildHistogramData(projectedBins)
    .map((bin) => {
      const x = Number(bin?.x);
      const y = Number(bin?.value);
      const err = Number(bin?.error);
      if (!Number.isFinite(x) || !Number.isFinite(y) || !Number.isFinite(err) || err <= 0) return null;
      const yLowRaw = y - Math.abs(err);
      const yHighRaw = y + Math.abs(err);
      const yLow = yScale === "log" ? signedLog10(yLowRaw) : yLowRaw;
      const yHigh = yScale === "log" ? signedLog10(yHighRaw) : yHighRaw;
      if (xScale === "log" && x <= 0) return null;
      return [x, yLow, yHigh];
    })
    .filter(Boolean);

  return { valueStep, relativeStep, absError };
};

export const buildHistogramStepData = (bins) => {
  const orderedBins = asArray(bins)
    .slice()
    .sort((left, right) => left.start - right.start);
  const points = [];
  for (const [index, bin] of orderedBins.entries()) {
    const rangeLabel = `${formatScientific(bin.start, 4)} → ${formatScientific(bin.stop, 4)}`;
    points.push({ x: bin.start, y: bin.value, error: bin.error, rangeLabel });
    points.push({ x: bin.stop, y: bin.value, error: bin.error, rangeLabel });
    const nextBin = orderedBins[index + 1];
    if (nextBin && nextBin.start !== bin.stop) {
      points.push({
        x: bin.stop,
        y: nextBin.value,
        error: nextBin.error,
        rangeLabel: `${formatScientific(nextBin.start, 4)} → ${formatScientific(nextBin.stop, 4)}`,
      });
    }
  }
  return points.filter((point) => Number.isFinite(point.x) && Number.isFinite(point.y));
};

export const buildHistogramRenderData = (bins, scale) => {
  const stepData = buildHistogramStepData(bins);
  if (scale !== "log") return stepData;
  return stepData.map((point) => ({
    ...point,
    raw_y: Number(point.y),
    y: signedLog10(point.y),
    error: Number.isFinite(point.error) ? point.error : 0,
  }));
};

export const buildRelativeErrorStepData = (bins) =>
  buildHistogramStepData(bins)
    .map((point) => {
      const value = Number(point?.y);
      const error = Number(point?.error);
      if (!Number.isFinite(value) || !Number.isFinite(error) || value === 0) {
        return { ...point, relative_error: null, positive_relative_error: null, negative_relative_error: null };
      }
      const relativeError = Math.abs(error / value);
      return {
        ...point,
        relative_error: relativeError,
        positive_relative_error: relativeError,
        negative_relative_error: -relativeError,
      };
    })
    .filter((point) => Number.isFinite(point.x));

export const buildDiscreteRelativeErrorData = (bins) =>
  asArray(bins).map((bin, index) => {
    const value = Number(bin?.value);
    const error = Number(bin?.error);
    if (!Number.isFinite(value) || !Number.isFinite(error) || value === 0) {
      return { index, relative_error: null, positive_relative_error: null, negative_relative_error: null };
    }
    const relativeError = Math.abs(error / value);
    return {
      index,
      relative_error: relativeError,
      positive_relative_error: relativeError,
      negative_relative_error: -relativeError,
    };
  });

export const discreteHistogramBinKey = (bin, index) => {
  if (bin?.label != null) return `label:${String(bin.label)}`;
  if (bin?.bin_id != null) return `id:${String(bin.bin_id)}`;
  const start = Number(bin?.start);
  if (Number.isFinite(start)) return `start:${start}`;
  return `index:${index}`;
};

export const histogramIsDiscrete = (histogram) =>
  asArray(histogram?.bins).some((bin) => bin && (bin.label != null || bin.bin_id != null));

export const parseUploadedHistogramBundle = (payload) => {
  if (!isObject(payload)) throw new Error("bundle payload must be a JSON object");
  const histograms = payload?.histograms;
  if (!isObject(histograms)) throw new Error("bundle payload must contain a 'histograms' object");
  const entries = Object.entries(histograms).filter(
    ([name, histogram]) => typeof name === "string" && name.trim().length > 0 && isObject(histogram),
  );
  if (entries.length === 0) throw new Error("bundle payload contains no valid histograms");
  const normalizedHistograms = {};
  entries.forEach(([name, histogram]) => {
    normalizedHistograms[name] = normalizeUploadedHistogram(name, histogram);
  });
  return {
    primaryHistogramName: typeof payload?.primary_histogram_name === "string" ? payload.primary_histogram_name : null,
    histograms: normalizedHistograms,
  };
};

const normalizeUploadedHistogram = (name, histogram) => {
  const bins = asArray(histogram?.bins);
  if (bins.length === 0) throw new Error(`histogram '${name}' must contain at least one bin`);

  const isNormalizedBins = bins.every(
    (bin) =>
      isObject(bin) &&
      Number.isFinite(Number(bin.start)) &&
      Number.isFinite(Number(bin.stop)) &&
      Number.isFinite(Number(bin.value)),
  );

  const normalizedBins = isNormalizedBins
    ? bins.map((bin, index) => {
        const start = Number(bin.start);
        const stop = Number(bin.stop);
        const value = Number(bin.value);
        const error = Number(bin.error);
        if (!Number.isFinite(start) || !Number.isFinite(stop) || !Number.isFinite(value)) {
          throw new Error(`histogram '${name}' contains invalid normalized bin at index ${index}`);
        }
        return {
          start,
          stop,
          value,
          error: Number.isFinite(error) ? Math.abs(error) : 0,
          label: bin?.label ?? null,
          bin_id: bin?.bin_id ?? null,
          entry_count: bin?.entry_count ?? null,
          sum_weights: bin?.sum_weights ?? null,
          sum_weights_squared: bin?.sum_weights_squared ?? null,
          mitigated_fill_count: bin?.mitigated_fill_count ?? null,
        };
      })
    : (() => {
        const sampleCount = Number(histogram?.sample_count);
        const hasGammaLoopBinShape = bins.every(
          (bin) =>
            isObject(bin) &&
            Number.isFinite(Number(bin?.x_min)) &&
            Number.isFinite(Number(bin?.x_max)) &&
            Number.isFinite(Number(bin?.sum_weights)) &&
            Number.isFinite(Number(bin?.sum_weights_squared)),
        );
        if (!hasGammaLoopBinShape || !Number.isFinite(sampleCount) || sampleCount < 0) {
          throw new Error(`histogram '${name}' uses unsupported bin syntax`);
        }
        return normalizeGammaLoopHistogramBins(histogram);
      })();

  if (
    normalizedBins.some(
      (bin) =>
        !Number.isFinite(Number(bin?.start)) ||
        !Number.isFinite(Number(bin?.stop)) ||
        !Number.isFinite(Number(bin?.value)) ||
        !Number.isFinite(Number(bin?.error)),
    )
  ) {
    throw new Error(`histogram '${name}' contains non-finite bin values`);
  }

  return {
    title: typeof histogram?.title === "string" && histogram.title.trim() ? histogram.title : name,
    bins: normalizedBins.map((bin) => ({
      start: Number(bin.start),
      stop: Number(bin.stop),
      value: Number(bin.value),
      error: Math.abs(Number(bin.error)),
      label: bin?.label ?? null,
      bin_id: bin?.bin_id ?? null,
      entry_count: bin?.entry_count ?? null,
      sum_weights: bin?.sum_weights ?? null,
      sum_weights_squared: bin?.sum_weights_squared ?? null,
      mitigated_fill_count: bin?.mitigated_fill_count ?? null,
    })),
  };
};

export const histogramSelectionKey = (histogramName) =>
  typeof histogramName === "string" && histogramName.trim().length > 0 ? histogramName : "__default__";

export const normalizeHistogramSortMode = (value) => {
  if (value === HISTOGRAM_SORT_BY_VALUE) return HISTOGRAM_SORT_BY_VALUE;
  if (value === HISTOGRAM_SORT_BY_ABS_VALUE) return HISTOGRAM_SORT_BY_ABS_VALUE;
  return HISTOGRAM_SORT_CANONICAL;
};

export const sortHistogramBinsByMode = (bins, sortMode) => {
  const normalizedMode = normalizeHistogramSortMode(sortMode);
  if (normalizedMode === HISTOGRAM_SORT_CANONICAL) {
    return asArray(bins)
      .map((bin, index) => ({ bin, index, key: String(bin?.label ?? bin?.bin_id ?? index) }))
      .sort((left, right) => {
        const compared = left.key.localeCompare(right.key, undefined, { numeric: true });
        return compared || left.index - right.index;
      })
      .map((entry) => entry.bin);
  }
  return asArray(bins)
    .map((bin, index) => ({ bin, index, value: Number(bin?.value) }))
    .sort((left, right) => {
      const leftValue = Number.isFinite(left.value)
        ? normalizedMode === HISTOGRAM_SORT_BY_ABS_VALUE
          ? Math.abs(left.value)
          : left.value
        : Number.NEGATIVE_INFINITY;
      const rightValue = Number.isFinite(right.value)
        ? normalizedMode === HISTOGRAM_SORT_BY_ABS_VALUE
          ? Math.abs(right.value)
          : right.value
        : Number.NEGATIVE_INFINITY;
      if (leftValue !== rightValue) return rightValue - leftValue;
      return left.index - right.index;
    })
    .map((entry) => entry.bin);
};

export const normalizeHistogramSelectionState = (bundle, histogramName) => {
  const key = histogramSelectionKey(histogramName);
  const stored = isObject(bundle?.selectionsByHistogram) ? bundle.selectionsByHistogram[key] : null;
  if (isObject(stored)) {
    const selectedHistograms = asArray(stored.selectedHistograms)
      .filter((value) => typeof value === "string")
      .filter((value, index, values) => values.indexOf(value) === index);
    const discreteAlignmentByHistogram = isObject(stored.discreteAlignmentByHistogram)
      ? Object.fromEntries(
          Object.entries(stored.discreteAlignmentByHistogram)
            .filter(([name, value]) => typeof name === "string" && typeof value === "string")
            .map(([name, value]) => [name, value === "by_index" ? "by_index" : "by_key"]),
        )
      : Object.fromEntries(selectedHistograms.map((name) => [name, "by_key"]));
    const sortModeByHistogram = isObject(stored.sortModeByHistogram)
      ? Object.fromEntries(
          Object.entries(stored.sortModeByHistogram)
            .filter(([name]) => typeof name === "string")
            .map(([name, value]) => [name, normalizeHistogramSortMode(value)]),
        )
      : Object.fromEntries(selectedHistograms.map((name) => [name, HISTOGRAM_SORT_CANONICAL]));
    return { key, selectedHistograms, discreteAlignmentByHistogram, sortModeByHistogram };
  }
  const fallbackSelection =
    typeof histogramName === "string" && isObject(bundle?.histograms) && isObject(bundle.histograms[histogramName])
      ? [histogramName]
      : [];
  return {
    key,
    selectedHistograms: fallbackSelection,
    discreteAlignmentByHistogram: Object.fromEntries(fallbackSelection.map((name) => [name, "by_key"])),
    sortModeByHistogram: Object.fromEntries(fallbackSelection.map((name) => [name, HISTOGRAM_SORT_CANONICAL])),
  };
};

export const buildHistogramYDomain = (bins, scale, visibleXRange = null) => {
  const valuesInRange = asArray(bins)
    .filter((bin) => {
      if (!visibleXRange) return true;
      const x = Number(bin?.x);
      return Number.isFinite(x) && x >= visibleXRange.min && x <= visibleXRange.max;
    })
    .flatMap((bin) => [
      Number(bin?.y ?? bin?.value) - Number(bin?.error || 0),
      Number(bin?.y ?? bin?.value) + Number(bin?.error || 0),
      Number(bin?.y ?? bin?.value),
    ])
    .filter((value) => Number.isFinite(value));
  const values =
    valuesInRange.length > 0
      ? valuesInRange
      : asArray(bins)
          .flatMap((bin) => [
            Number(bin?.y ?? bin?.value) - Number(bin?.error || 0),
            Number(bin?.y ?? bin?.value) + Number(bin?.error || 0),
            Number(bin?.y ?? bin?.value),
          ])
          .filter((value) => Number.isFinite(value));
  if (values.length === 0) return ["auto", "auto"];
  if (scale === "log") return fitDomain(values.map((entry) => signedLog10(entry)).filter(Number.isFinite));
  return fitDomain(values);
};

export const buildRelativeErrorYDomain = (points, visibleXRange = null) => {
  const selectedPoints = asArray(points).filter((point) => {
    if (!visibleXRange) return true;
    const x = Number(point?.x ?? point?.index);
    return Number.isFinite(x) && x >= visibleXRange.min && x <= visibleXRange.max;
  });
  const sourcePoints = selectedPoints.length > 0 ? selectedPoints : asArray(points);
  const maxRelativeError = Math.max(
    0,
    ...sourcePoints.map((point) => Number(point?.relative_error)).filter((value) => Number.isFinite(value)),
  );
  if (maxRelativeError <= 0) return [-1, 1];
  const padded = maxRelativeError * 1.08;
  return [-padded, padded];
};
