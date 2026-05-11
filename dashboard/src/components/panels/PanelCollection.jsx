import { Suspense, forwardRef, lazy, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Alert,
  Box,
  Card,
  CardContent,
  Button,
  FormControl,
  FormControlLabel,
  Switch,
  MenuItem,
  Stack,
  Select,
  Slider,
  Typography,
} from "@mui/material";
import { formatCompactNumber, formatDateTime, formatScientific } from "../../utils/formatters";
import { asArray } from "../../utils/collections";
import {
  KeyValuePanel,
  ProgressPanel,
  SelectPanel,
  SvgPanel,
  TextPanel,
  TickBreakdownPanel,
  renderStructuredValue,
} from "./BasicPanels";
import FigureExportActions, { escapeXml } from "./FigureExportActions";
import TablePanel from "./TablePanel";
import {
  HISTOGRAM_NEGATIVE_COLOR,
  HISTOGRAM_MODE_CDF,
  HISTOGRAM_MODE_PDF,
  HISTOGRAM_POSITIVE_COLOR,
  HISTOGRAM_SORT_BY_ABS_VALUE,
  HISTOGRAM_SORT_BY_VALUE,
  HISTOGRAM_SORT_CANONICAL,
  HISTOGRAM_ZERO_COLOR,
  buildCdfBins,
  buildDiscreteRelativeErrorData,
  buildHistogramData,
  buildHistogramRenderData,
  buildHistogramStepData,
  buildHistogramYDomain,
  buildRelativeErrorStepData,
  buildRelativeErrorYDomain,
  discreteHistogramBinKey,
  fitDomain,
  fitHistogramXDomain,
  fitXDomain,
  formatSignedLogAxisValue,
  histogramIsDiscrete,
  histogramSelectionKey,
  histogramSignColorFromRaw,
  normalizeHistogramMode,
  normalizeGammaLoopHistogramBins,
  normalizeHistogramSelectionState,
  normalizeHistogramSortMode,
  parseUploadedHistogramBundle,
  readHistogramBundleView,
  projectOverlayHistogramToReferenceBins,
  readHistogramBundleSelectedValue,
  readHistogramScaleFromPanelValue,
  readHistogramYZoomFromPanelValue,
  readHistogramZoomFromPanelValue,
  signedLog10,
  sortHistogramBinsByMode,
  writeHistogramBundlePanelValue,
} from "./histogramUtils";
import {
  FULL_ZOOM,
  HISTORY_X_AXIS_MODE_COMPLETED_SAMPLES,
  HISTORY_X_AXIS_MODE_SAMPLER_UPTIME,
  HISTORY_X_AXIS_MODE_WALL_TIME,
  buildDataZoom,
  extractSharedHistoryView,
  extractSharedImageZoom,
  isObject,
  isSharedHistoryTimeseriesPanelSpec,
  mergeSharedHistoryView,
  mergeSharedImageZoom,
  normalizeZoomRange,
  readDataZoomRanges,
  readHistoryXAxisModeFromPanelValue,
  readTailPinnedFromPanelValue,
  readYZoomFromPanelValue,
  readZoomFromPanelValue,
  visibleXRangeFromZoom,
  visibleXRangeFromZoomWithScale,
  writeZoomPanelValue,
  zoomRangeChanged,
} from "./panelView";

const ReactECharts = lazy(() =>
  Promise.all([import("echarts-for-react"), import("../../lib/echarts")]).then(([module]) => ({
    default: module.default,
  })),
);

const LazyChart = forwardRef((props, ref) => (
  <Suspense
    fallback={
      <Box
        sx={{
          width: "100%",
          height: "100%",
          minHeight: 160,
          display: "grid",
          placeItems: "center",
          color: "text.secondary",
          typography: "body2",
        }}
      >
        Loading chart...
      </Box>
    }
  >
    <ReactECharts ref={ref} {...props} />
  </Suspense>
));
LazyChart.displayName = "LazyChart";

const PANEL_ORDER_RANK = new Map([
  ["sample_progress", 0],
  ["estimate_summary", 1],
  ["real_estimate_history", 3],
  ["imag_estimate_history", 4],
  ["abs_signal_to_noise_history", 5],
  ["gammaloop_histogram_bundle", 20],
  ["gammaloop_histogram_bundle_selected", 21],
  ["gammaloop_selected_histogram", 21],
  ["gammaloop_evaluation_timing", 22],
  ["gammaloop_evaluation_diagnostics", 23],
]);

const sortRenderablePanels = (panels) =>
  asArray(panels)
    .map((panel, index) => ({ panel, index }))
    .sort((left, right) => {
      const leftId = left.panel?.descriptor?.panel_id;
      const rightId = right.panel?.descriptor?.panel_id;
      const leftRank = PANEL_ORDER_RANK.get(leftId) ?? Number.MAX_SAFE_INTEGER;
      const rightRank = PANEL_ORDER_RANK.get(rightId) ?? Number.MAX_SAFE_INTEGER;
      if (leftRank !== rightRank) return leftRank - rightRank;
      return left.index - right.index;
    })
    .map(({ panel }) => panel);

const replacePanelPairWithOverlay = (panels, firstId, secondId, overlayPanel) => {
  const firstIndex = panels.findIndex((panel) => panel?.descriptor?.panel_id === firstId);
  const secondIndex = panels.findIndex((panel) => panel?.descriptor?.panel_id === secondId);
  if (firstIndex < 0 || secondIndex < 0) return panels;
  const first = panels[firstIndex];
  const second = panels[secondIndex];
  if (!first?.state || !second?.state) return panels;

  const next = [...panels];
  const low = Math.min(firstIndex, secondIndex);
  const high = Math.max(firstIndex, secondIndex);
  next.splice(high, 1);
  next.splice(low, 1, overlayPanel);
  return next;
};

const normalizePanelPoints = (points) =>
  asArray(points)
    .map((point) => ({
      x: Number(point?.x),
      y: Number(point?.y),
      x_sampler_uptime_ms: Number.isFinite(Number(point?.x_sampler_uptime_ms))
        ? Number(point?.x_sampler_uptime_ms)
        : null,
      x_completed_samples_total: Number.isFinite(Number(point?.x_completed_samples_total))
        ? Number(point?.x_completed_samples_total)
        : null,
      y_min: Number.isFinite(Number(point?.y_min)) ? Number(point?.y_min) : null,
      y_max: Number.isFinite(Number(point?.y_max)) ? Number(point?.y_max) : null,
    }))
    .filter((point) => Number.isFinite(point.x) && Number.isFinite(point.y));

const buildPdfLineOverlayPanel = (integrandPanel, pdfPanel) => ({
  descriptor: {
    panel_id: "pdf_adaptation_integrand_pdf_line_overlay",
    label: "Normalized Integrand vs PDF (1D)",
    kind: "multi_timeseries",
    history: "none",
    width: "full",
  },
  state: {
    panel_id: "pdf_adaptation_integrand_pdf_line_overlay",
    series: [
      {
        id: "integrand",
        label: "Normalized integrand",
        color: "#005f73",
        smooth: false,
        points: normalizePanelPoints(integrandPanel?.state?.points),
      },
      {
        id: "pdf",
        label: "Normalized PDF",
        color: "#bb3e03",
        smooth: false,
        points: normalizePanelPoints(pdfPanel?.state?.points),
      },
    ].filter((series) => asArray(series.points).length > 0),
  },
  value: null,
});

const buildPdfHistogramOverlayPanel = ({
  panelId,
  label,
  primaryPanel,
  overlayPanel,
  overlayName,
  overlayColor,
}) => ({
  descriptor: {
    panel_id: panelId,
    label,
    kind: "histogram",
    history: "none",
    width: "full",
  },
  state: {
    ...(isObject(primaryPanel?.state) ? primaryPanel.state : {}),
    panel_id: panelId,
    source_panel_id: panelId,
    name: null,
    overlay_histograms: [
      {
        name: overlayName,
        color: overlayColor,
        bins: asArray(overlayPanel?.state?.bins),
      },
    ],
  },
  value: null,
});

const buildRenderablePanels = (panelSpecs, panelStates, panelValues) => {
  const stateMap = new Map(asArray(panelStates).map((panel) => [panel.panel_id, panel]));
  let renderablePanels = asArray(panelSpecs).map((spec) => ({
    descriptor: spec,
    state: stateMap.get(spec.panel_id) || null,
    value: panelValues?.[spec.panel_id],
  }));
  const bundlePanel = renderablePanels.find(({ descriptor, state }) => {
    const payload = state?.payload;
    return (
      descriptor?.kind === "table" &&
      payload?.histograms &&
      typeof payload.histograms === "object" &&
      !Array.isArray(payload.histograms)
    );
  });
  const payload = bundlePanel?.state?.payload;
  const histograms = payload?.histograms;
  if (bundlePanel && histograms && typeof histograms === "object" && !Array.isArray(histograms)) {
    const sourcePanelId = bundlePanel?.descriptor?.panel_id || "histogram_bundle";
    const selectedFromValue = readHistogramBundleSelectedValue(bundlePanel.value);
    const selectedName =
      selectedFromValue ??
      payload?.primary_histogram_name ??
      Object.keys(histograms).find((key) => key && typeof histograms[key] === "object") ??
      null;
    const selectedHistogram =
      (selectedName && histograms[selectedName]) ||
      (payload?.primary_histogram_name && histograms[payload.primary_histogram_name]) ||
      Object.values(histograms).find((entry) => entry && typeof entry === "object") ||
      null;
    if (selectedHistogram) {
      const normalizedBins = asArray(selectedHistogram?.bins).some((bin) => bin?.value != null)
        ? buildHistogramData(selectedHistogram.bins)
        : normalizeGammaLoopHistogramBins(selectedHistogram);
      renderablePanels.push({
        descriptor: {
          panel_id: `${sourcePanelId}_selected`,
          label: "Selected Histogram",
          kind: "histogram",
          history: "none",
          width: "full",
        },
        state: {
          panel_id: `${sourcePanelId}_selected`,
          source_panel_id: sourcePanelId,
          name: selectedName,
          title: selectedHistogram.title,
          type_description: selectedHistogram.type_description,
          phase: selectedHistogram.phase,
          value_transform: selectedHistogram.value_transform,
          sample_count: selectedHistogram.sample_count,
          x_min: selectedHistogram.x_min,
          x_max: selectedHistogram.x_max,
          discrete_ordering: selectedHistogram.discrete_ordering,
          log_x_axis: selectedHistogram.log_x_axis,
          log_y_axis: selectedHistogram.log_y_axis,
          bins: normalizedBins,
        },
        value: bundlePanel.value ?? null,
      });
    }
  }

  const lineIntegrandPanel = renderablePanels.find(
    ({ descriptor }) => descriptor?.panel_id === "pdf_adaptation_log_integrand_line",
  );
  const linePdfPanel = renderablePanels.find(({ descriptor }) => descriptor?.panel_id === "pdf_adaptation_log_pdf_line");
  if (lineIntegrandPanel?.state && linePdfPanel?.state) {
    renderablePanels = replacePanelPairWithOverlay(
      renderablePanels,
      "pdf_adaptation_log_integrand_line",
      "pdf_adaptation_log_pdf_line",
      buildPdfLineOverlayPanel(lineIntegrandPanel, linePdfPanel),
    );
  }

  const histogramIntegrandPanel = renderablePanels.find(
    ({ descriptor }) => descriptor?.panel_id === "pdf_adaptation_log_integrand_histogram",
  );
  const histogramPdfPanel = renderablePanels.find(
    ({ descriptor }) => descriptor?.panel_id === "pdf_adaptation_log_pdf_histogram",
  );
  if (histogramIntegrandPanel?.state && histogramPdfPanel?.state) {
    renderablePanels = replacePanelPairWithOverlay(
      renderablePanels,
      "pdf_adaptation_log_integrand_histogram",
      "pdf_adaptation_log_pdf_histogram",
      buildPdfHistogramOverlayPanel({
        panelId: "pdf_adaptation_integrand_pdf_histogram_overlay",
        label: "Histogram: Normalized Integrand vs PDF",
        primaryPanel: histogramIntegrandPanel,
        overlayPanel: histogramPdfPanel,
        overlayName: "Normalized PDF",
        overlayColor: "#bb3e03",
      }),
    );
  }

  const histogramOversamplingPanel = renderablePanels.find(
    ({ descriptor }) => descriptor?.panel_id === "pdf_adaptation_oversampling_histogram",
  );
  const histogramOversamplingNormalizedPanel = renderablePanels.find(
    ({ descriptor }) => descriptor?.panel_id === "pdf_adaptation_oversampling_plane_normalized_histogram",
  );
  if (histogramOversamplingPanel?.state && histogramOversamplingNormalizedPanel?.state) {
    renderablePanels = replacePanelPairWithOverlay(
      renderablePanels,
      "pdf_adaptation_oversampling_histogram",
      "pdf_adaptation_oversampling_plane_normalized_histogram",
      buildPdfHistogramOverlayPanel({
        panelId: "pdf_adaptation_oversampling_histogram_overlay",
        label: "Histogram: Sampling Accuracy (Plane) vs Sampling Accuracy (Global)",
        primaryPanel: histogramOversamplingPanel,
        overlayPanel: histogramOversamplingNormalizedPanel,
        overlayName: "Sampling accuracy (global)",
        overlayColor: "#6a994e",
      }),
    );
  }

  return sortRenderablePanels(renderablePanels);
};

const inferXAxisLabel = (panelId) => (String(panelId || "").includes("_history") ? "Nr samples" : null);
const inferNumericXAxisLabel = (panelId, mode = HISTORY_X_AXIS_MODE_WALL_TIME) => {
  if (mode === HISTORY_X_AXIS_MODE_SAMPLER_UPTIME) return "Sampler Runner Uptime";
  if (mode === HISTORY_X_AXIS_MODE_COMPLETED_SAMPLES) return "Completed Samples";
  return inferXAxisLabel(panelId) || "x";
};
const TIMESTAMP_X_THRESHOLD_MS = 1e11;
const isTimestampDomain = (domain) => {
  const [min, max] = asArray(domain);
  return Number.isFinite(min) && Number.isFinite(max) && min >= 0 && max >= TIMESTAMP_X_THRESHOLD_MS;
};
const formatElapsedTime = (elapsedMs) => {
  const totalSeconds = Math.max(0, Math.round(Number(elapsedMs) / 1000));
  if (!Number.isFinite(totalSeconds)) return "n/a";
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) {
    return `${String(hours).padStart(2, "0")}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
  }
  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
};
const formatAbsoluteLocalTime = (timestampMs) => {
  const date = new Date(Number(timestampMs));
  return Number.isNaN(date.getTime()) ? "n/a" : date.toLocaleString();
};
const formatTimeseriesXAxisValue = (value, mode, isTimestamp, originMs) => {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return "n/a";
  if (mode === HISTORY_X_AXIS_MODE_SAMPLER_UPTIME) return formatElapsedTime(numeric);
  if (mode === HISTORY_X_AXIS_MODE_COMPLETED_SAMPLES) return formatAxisValue(numeric);
  return isTimestamp ? formatElapsedTime(numeric - originMs) : formatAxisValue(numeric);
};
const buildTimeseriesTooltipFormatter = (mode, isTimestamp, originMs) => (params) => {
  const entries = asArray(params);
  if (entries.length === 0) return "";
  const axisValue = Number(entries[0]?.axisValue);
  const header =
    mode === HISTORY_X_AXIS_MODE_SAMPLER_UPTIME
      ? escapeXml(formatElapsedTime(axisValue))
      : mode === HISTORY_X_AXIS_MODE_COMPLETED_SAMPLES
        ? escapeXml(formatAxisValue(axisValue))
        : isTimestamp
          ? `${formatElapsedTime(axisValue - originMs)}<br/>${escapeXml(formatAbsoluteLocalTime(axisValue))}`
          : escapeXml(formatAxisValue(axisValue));
  const lines = entries.map((entry) => {
    const rawValue = Array.isArray(entry?.value) ? entry.value[1] : entry?.value;
    return `${entry?.marker ?? ""}${escapeXml(entry?.seriesName ?? "")}: ${Number.isFinite(Number(rawValue)) ? formatScientific(Number(rawValue), 6) : "n/a"}`;
  });
  return [header, ...lines].join("<br/>");
};
const buildMultiSeriesData = (seriesList) => {
  const rows = new Map();
  for (const series of asArray(seriesList)) {
    for (const point of asArray(series.points)) {
      const row = rows.get(point.x) || { x: point.x };
      row[series.id] = point.y;
      rows.set(point.x, row);
    }
  }
  return Array.from(rows.values()).sort((a, b) => a.x - b.x);
};

const historyXAxisValueForPoint = (point, mode) => {
  if (mode === HISTORY_X_AXIS_MODE_SAMPLER_UPTIME) {
    const uptimeMs = Number(point?.x_sampler_uptime_ms);
    if (Number.isFinite(uptimeMs)) return uptimeMs;
  }
  if (mode === HISTORY_X_AXIS_MODE_COMPLETED_SAMPLES) {
    const completedSamples = Number(point?.x_completed_samples_total);
    if (Number.isFinite(completedSamples)) return completedSamples;
  }
  const fallback = Number(point?.x);
  return Number.isFinite(fallback) ? fallback : 0;
};

const remapTimeseriesPointXAxis = (point, mode) => ({
  ...point,
  x: historyXAxisValueForPoint(point, mode),
});

const remapAndSortTimeseriesPoints = (points, mode) =>
  asArray(points)
    .map((point) => remapTimeseriesPointXAxis(point, mode))
    .sort((left, right) => Number(left?.x) - Number(right?.x));

const lineColors = ["#005f73", "#bb3e03", "#0a9396", "#ae2012", "#ca6702"];
const histogramOverlayColors = ["#9b2226", "#3a86ff", "#ff006e", "#6a994e", "#ff7f11", "#8338ec"];

const scalarHeatmapColors = ["rgb(0,0,255)", "rgb(128,200,128)", "rgb(255,0,0)"];

const panelColumnSpan = (descriptor) => {
  switch (descriptor?.width) {
    case "compact":
      return { xs: "1 / -1", md: "span 4" };
    case "full":
      return { xs: "1 / -1", md: "1 / -1" };
    case "half":
      return { xs: "1 / -1", md: "span 6" };
    default:
      switch (descriptor?.kind) {
        case "scalar_timeseries":
        case "multi_timeseries":
        case "tick_breakdown":
        case "svg":
        case "image2d":
        case "table":
        case "histogram":
          return { xs: "1 / -1", md: "1 / -1" };
        case "progress":
        case "key_value":
        case "text":
        case "select":
        default:
          return { xs: "1 / -1", md: "span 6" };
      }
  }
};

const gridColor = "rgba(148,163,184,0.18)";
const DISCRETE_BAR_CATEGORY_GAP = "30%";
const DISCRETE_BAR_GAP = "30%";

const formatAxisValue = (value) => {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? formatScientific(numeric, 3) : "";
};

const formatCategoryAxisValue = (value) => {
  if (value == null) return "";
  const text = String(value).trim();
  return text.length > 0 ? text : "";
};

const EDGE_EPSILON = 1e-9;

const nearlyEqual = (left, right, epsilon = EDGE_EPSILON) => {
  const a = Number(left);
  const b = Number(right);
  if (!Number.isFinite(a) || !Number.isFinite(b)) return false;
  return Math.abs(a - b) <= epsilon * Math.max(1, Math.abs(a), Math.abs(b));
};

const binsShareEdges = (referenceBins, overlayBins) => {
  if (referenceBins.length !== overlayBins.length) return false;
  for (let i = 0; i < referenceBins.length; i += 1) {
    const left = referenceBins[i];
    const right = overlayBins[i];
    if (!nearlyEqual(left?.start, right?.start) || !nearlyEqual(left?.stop, right?.stop)) return false;
  }
  return true;
};

const buildOverlaySeriesFromBins = (canonicalBins, yScale, xScale) => ({
  valueStep: buildHistogramRenderData(canonicalBins, yScale)
    .map((point) => [Number(point?.x), Number(point?.y)])
    .filter(([x]) => Number.isFinite(x) && (xScale !== "log" || x > 0)),
  relativeStep: buildRelativeErrorStepData(canonicalBins)
    .map((point) => [Number(point?.x), Number(point?.relative_error)])
    .filter(([x]) => Number.isFinite(x) && (xScale !== "log" || x > 0)),
  absError: canonicalBins
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
    .filter(Boolean),
});

const sampleHistogramBinAtX = (bins, x) => {
  const numericX = Number(x);
  if (!Number.isFinite(numericX)) return null;
  for (const bin of asArray(bins)) {
    const start = Number(bin?.start);
    const stop = Number(bin?.stop);
    if (!Number.isFinite(start) || !Number.isFinite(stop)) continue;
    const contains = numericX >= start - EDGE_EPSILON && numericX <= stop + EDGE_EPSILON;
    if (contains) return bin;
  }
  return null;
};

const projectBinsToReferenceBins = (referenceBins, overlayBins) =>
  asArray(referenceBins).map((referenceBin) => {
    const matched = sampleHistogramBinAtX(overlayBins, referenceBin?.x);
    if (!matched) return null;
    return {
      start: referenceBin.start,
      stop: referenceBin.stop,
      x: referenceBin.x,
      value: Number(matched.value),
      error: Number.isFinite(Number(matched.error)) ? Math.abs(Number(matched.error)) : 0,
    };
  });

const buildCdfBinsPreservingNulls = (bins) => {
  let cumulativeValue = 0;
  let cumulativeVariance = 0;
  return asArray(bins).map((bin) => {
    if (!bin) return null;
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

const buildLogRatioPoints = (referenceBins, overlayBins, isDiscrete, xScale) =>
  asArray(referenceBins)
    .map((referenceBin, index) => {
      const overlayBin = asArray(overlayBins)[index];
      const numerator = Number(referenceBin?.value);
      const denominator = Number(overlayBin?.value);
      const numeratorError = Math.abs(Number(referenceBin?.error));
      const denominatorError = Math.abs(Number(overlayBin?.error));
      if (!Number.isFinite(numerator) || !Number.isFinite(denominator) || numerator === 0 || denominator === 0) {
        return null;
      }
      const ratio = numerator / denominator;
      if (!Number.isFinite(ratio) || ratio <= 0) return null;
      const relativeNumeratorError =
        Number.isFinite(numeratorError) && numerator !== 0 ? numeratorError / Math.abs(numerator) : 0;
      const relativeDenominatorError =
        Number.isFinite(denominatorError) && denominator !== 0 ? denominatorError / Math.abs(denominator) : 0;
      const logRatio = Math.log10(ratio);
      const logRatioError = Math.hypot(relativeNumeratorError, relativeDenominatorError) / Math.LN10;
      const x = isDiscrete ? index : Number(referenceBin?.x);
      if (!Number.isFinite(x) || (!isDiscrete && xScale === "log" && x <= 0)) return null;
      return [x, logRatio, logRatio - logRatioError, logRatio + logRatioError];
    })
    .filter(Boolean);

const clampHeatmapSpread = (candidate, fallback = 1) => {
  const numeric = Number(candidate);
  if (!Number.isFinite(numeric)) return fallback;
  return Math.max(0.25, Math.min(4, numeric));
};
const readHeatmapSpreadFromPanelValue = (value, fallback = 1) =>
  clampHeatmapSpread(isObject(value) ? value.spread : null, fallback);
const writeHeatmapSpreadPanelValue = (current, spread) => {
  const next = isObject(current) ? { ...current } : {};
  next.spread = clampHeatmapSpread(spread, 1);
  return next;
};

const inferDefaultHistogramYScale = (_state) => "linear";

const inferDefaultHistogramXScale = (state) => (state?.log_x_axis ? "log" : "linear");

const isPdfAdaptationImagePanelSpec = (spec) =>
  spec?.kind === "image2d" &&
  typeof spec?.panel_id === "string" &&
  spec.panel_id.startsWith("pdf_adaptation_");

const baseCartesianGrid = {
  left: 56,
  right: 20,
  top: 12,
  bottom: 48,
};

const baseAxisLabel = {
  color: "#64748b",
  fontSize: 12,
  formatter: (value) => formatAxisValue(value),
};

const buildErrorBarSeries = ({ name = "error", data, color = "#7c8a96", capPx = 4 }) => ({
  type: "custom",
  name,
  data,
  clip: true,
  silent: true,
  z: 5,
  tooltip: { show: false },
  renderItem: (params, api) => {
    const xValue = Number(api.value(0));
    const yLowValue = Number(api.value(1));
    const yHighValue = Number(api.value(2));
    if (!Number.isFinite(xValue) || !Number.isFinite(yLowValue) || !Number.isFinite(yHighValue)) {
      return null;
    }
    const [xPx, yLowPx] = api.coord([xValue, yLowValue]);
    const [, yHighPx] = api.coord([xValue, yHighValue]);
    if (!Number.isFinite(xPx) || !Number.isFinite(yLowPx) || !Number.isFinite(yHighPx)) {
      return null;
    }
    const coordSys = params?.coordSys;
    if (!coordSys) return null;
    const left = Number(coordSys.x);
    const right = Number(coordSys.x) + Number(coordSys.width);
    const top = Number(coordSys.y);
    const bottom = Number(coordSys.y) + Number(coordSys.height);
    if (!Number.isFinite(left) || !Number.isFinite(right) || !Number.isFinite(top) || !Number.isFinite(bottom)) {
      return null;
    }
    if (xPx < left || xPx > right) return null;
    if ((yLowPx < top && yHighPx < top) || (yLowPx > bottom && yHighPx > bottom)) return null;
    const y1 = Math.max(top, Math.min(bottom, yLowPx));
    const y2 = Math.max(top, Math.min(bottom, yHighPx));
    const capLeft = Math.max(left, xPx - capPx);
    const capRight = Math.min(right, xPx + capPx);
    return {
      type: "group",
      children: [
        {
          type: "line",
          shape: { x1: xPx, y1, x2: xPx, y2 },
          style: { stroke: color, lineWidth: 1.2 },
        },
        {
          type: "line",
          shape: { x1: capLeft, y1, x2: capRight, y2: y1 },
          style: { stroke: color, lineWidth: 1.2 },
        },
        {
          type: "line",
          shape: { x1: capLeft, y1: y2, x2: capRight, y2 },
          style: { stroke: color, lineWidth: 1.2 },
        },
      ],
    };
  },
});

const buildDiscreteOffsetErrorBarSeries = ({
  name = "error",
  data,
  color = "#7c8a96",
  slotIndex = 0,
  slotCount = 1,
  barWidthRatio = 0.58,
  barCategoryGap = DISCRETE_BAR_CATEGORY_GAP,
  barGap = DISCRETE_BAR_GAP,
}) => ({
  type: "custom",
  name,
  data,
  clip: true,
  silent: true,
  z: 6,
  tooltip: { show: false },
  renderItem: (params, api) => {
    const xValue = Number(api.value(0));
    const yLowValue = Number(api.value(1));
    const yHighValue = Number(api.value(2));
    if (!Number.isFinite(xValue) || !Number.isFinite(yLowValue) || !Number.isFinite(yHighValue)) {
      return null;
    }
    const [baseXPx, yLowPx] = api.coord([xValue, yLowValue]);
    const [, yHighPx] = api.coord([xValue, yHighValue]);
    if (!Number.isFinite(baseXPx) || !Number.isFinite(yLowPx) || !Number.isFinite(yHighPx)) {
      return null;
    }
    const coordSys = params?.coordSys;
    if (!coordSys) return null;
    const left = Number(coordSys.x);
    const right = Number(coordSys.x) + Number(coordSys.width);
    const top = Number(coordSys.y);
    const bottom = Number(coordSys.y) + Number(coordSys.height);
    if (!Number.isFinite(left) || !Number.isFinite(right) || !Number.isFinite(top) || !Number.isFinite(bottom)) {
      return null;
    }
    const normalizedSlotCount = Math.max(1, Number(slotCount) || 1);
    const normalizedSlotIndex = Math.min(normalizedSlotCount - 1, Math.max(0, Number(slotIndex) || 0));
    const layouts = api.barLayout({
      count: normalizedSlotCount,
      barCategoryGap,
      barGap,
    });
    const selectedLayout = layouts[normalizedSlotIndex] || layouts[0];
    const slotPixelWidth = Math.max(1, Number(selectedLayout?.width) || Number(api.size([1, 0])?.[0]) || 1);
    const centerOffset = Number(selectedLayout?.offsetCenter ?? selectedLayout?.offset ?? 0);
    const xPx = baseXPx + centerOffset;
    const barWidth = Math.max(1, slotPixelWidth * barWidthRatio);
    const capHalf = Math.max(1, Math.min(6, barWidth * 0.5));
    if (xPx < left || xPx > right) return null;
    if ((yLowPx < top && yHighPx < top) || (yLowPx > bottom && yHighPx > bottom)) return null;
    const y1 = Math.max(top, Math.min(bottom, yLowPx));
    const y2 = Math.max(top, Math.min(bottom, yHighPx));
    const capLeft = Math.max(left, xPx - capHalf);
    const capRight = Math.min(right, xPx + capHalf);
    return {
      type: "group",
      children: [
        {
          type: "line",
          shape: { x1: xPx, y1, x2: xPx, y2 },
          style: { stroke: color, lineWidth: 1.1 },
        },
        {
          type: "line",
          shape: { x1: capLeft, y1, x2: capRight, y2: y1 },
          style: { stroke: color, lineWidth: 1.1 },
        },
        {
          type: "line",
          shape: { x1: capLeft, y1: y2, x2: capRight, y2 },
          style: { stroke: color, lineWidth: 1.1 },
        },
      ],
    };
  },
});

const buildDiscreteOffsetRangeBarSeries = ({
  name = "range",
  data,
  color = "rgba(187, 62, 3, 0.55)",
  widthRatio = 0.58,
  slotIndex = 0,
  slotCount = 1,
  barCategoryGap = DISCRETE_BAR_CATEGORY_GAP,
  barGap = DISCRETE_BAR_GAP,
}) => ({
  type: "custom",
  name,
  data,
  clip: true,
  silent: true,
  z: 4,
  tooltip: { show: false },
  renderItem: (params, api) => {
    const xValue = Number(api.value(0));
    const yLowValue = Number(api.value(1));
    const yHighValue = Number(api.value(2));
    if (!Number.isFinite(xValue) || !Number.isFinite(yLowValue) || !Number.isFinite(yHighValue)) {
      return null;
    }
    const [baseXPx, yLowPx] = api.coord([xValue, yLowValue]);
    const [, yHighPx] = api.coord([xValue, yHighValue]);
    if (!Number.isFinite(baseXPx) || !Number.isFinite(yLowPx) || !Number.isFinite(yHighPx)) {
      return null;
    }
    const coordSys = params?.coordSys;
    if (!coordSys) return null;
    const left = Number(coordSys.x);
    const right = Number(coordSys.x) + Number(coordSys.width);
    const top = Number(coordSys.y);
    const bottom = Number(coordSys.y) + Number(coordSys.height);
    if (!Number.isFinite(left) || !Number.isFinite(right) || !Number.isFinite(top) || !Number.isFinite(bottom)) {
      return null;
    }
    const normalizedSlotCount = Math.max(1, Number(slotCount) || 1);
    const normalizedSlotIndex = Math.min(normalizedSlotCount - 1, Math.max(0, Number(slotIndex) || 0));
    const layouts = api.barLayout({
      count: normalizedSlotCount,
      barCategoryGap,
      barGap,
    });
    const selectedLayout = layouts[normalizedSlotIndex] || layouts[0];
    const slotPixelWidth = Math.max(1, Number(selectedLayout?.width) || Number(api.size([1, 0])?.[0]) || 1);
    const centerOffset = Number(selectedLayout?.offsetCenter ?? selectedLayout?.offset ?? 0);
    const xPx = baseXPx + centerOffset;
    const barWidth = Math.max(1, slotPixelWidth * widthRatio);
    const xLeft = Math.max(left, xPx - barWidth / 2);
    const xRight = Math.min(right, xPx + barWidth / 2);
    if (xRight <= xLeft) return null;
    const y1 = Math.max(top, Math.min(bottom, yLowPx));
    const y2 = Math.max(top, Math.min(bottom, yHighPx));
    const rectTop = Math.min(y1, y2);
    const rectBottom = Math.max(y1, y2);
    if (rectBottom <= rectTop) return null;
    return {
      type: "rect",
      shape: { x: xLeft, y: rectTop, width: xRight - xLeft, height: rectBottom - rectTop },
      style: { fill: color, stroke: color, lineWidth: 0.6 },
    };
  },
});

const ScalarTimeseriesPanel = ({ title, state, value = undefined, onValueChange = null }) => {
  const figureRef = useRef(null);
  const echartsRef = useRef(null);
  const panelId = state?.panel_id || null;
  const isHistoryPanel = useMemo(() => String(panelId || "").includes("_history"), [panelId]);
  const historyXAxisMode = readHistoryXAxisModeFromPanelValue(
    value,
    isHistoryPanel ? HISTORY_X_AXIS_MODE_SAMPLER_UPTIME : HISTORY_X_AXIS_MODE_WALL_TIME,
  );
  const points = remapAndSortTimeseriesPoints(state?.points, historyXAxisMode);
  const meanData = points.map((point) => [Number(point?.x), Number(point?.y)]);
  const targetValue = Number(state?.target);
  const hasTargetLine = Number.isFinite(targetValue);
  const errorBarData = points
    .map((point) => {
      const x = Number(point?.x);
      const yMin = Number(point?.y_min);
      const yMax = Number(point?.y_max);
      if (!Number.isFinite(x) || !Number.isFinite(yMin) || !Number.isFinite(yMax) || yMax < yMin || yMax === yMin) {
        return null;
      }
      return [x, yMin, yMax];
    })
    .filter(Boolean);
  const domain = fitDomain([
    ...points.flatMap((point) => [point.y, point.y_min, point.y_max]),
    ...(hasTargetLine ? [targetValue] : []),
  ]);
  const xDomain = fitXDomain(points.map((point) => point.x));
  const zoomRange = readZoomFromPanelValue(value, FULL_ZOOM);
  const usesTimestampXAxis = useMemo(
    () => historyXAxisMode === HISTORY_X_AXIS_MODE_WALL_TIME && isTimestampDomain(xDomain),
    [historyXAxisMode, xDomain],
  );
  const xAxisOriginMs = useMemo(() => (usesTimestampXAxis ? Number(xDomain[0]) : 0), [usesTimestampXAxis, xDomain]);
  const tailPinned = readTailPinnedFromPanelValue(value, isHistoryPanel);
  const visibleXRange = useMemo(() => visibleXRangeFromZoom(xDomain, zoomRange), [xDomain, zoomRange]);
  const yZoomRange = readYZoomFromPanelValue(value, FULL_ZOOM);
  const visibleDomain = useMemo(() => {
    if (!visibleXRange) return domain;
    const inRangeValues = points
      .filter((point) => {
        const x = Number(point?.x);
        return Number.isFinite(x) && x >= visibleXRange.min && x <= visibleXRange.max;
      })
      .flatMap((point) => [point.y, point.y_min, point.y_max]);
    if (hasTargetLine) inRangeValues.push(targetValue);
    const fitted = fitDomain(inRangeValues);
    return inRangeValues.length > 0 ? fitted : domain;
  }, [domain, hasTargetLine, points, targetValue, visibleXRange]);
  const bandSegments = useMemo(() => {
    const segments = [];
    for (let index = 1; index < points.length; index += 1) {
      const left = points[index - 1];
      const right = points[index];
      const x1 = Number(left?.x);
      const x2 = Number(right?.x);
      const yMin1 = Number(left?.y_min);
      const yMin2 = Number(right?.y_min);
      const yMax1 = Number(left?.y_max);
      const yMax2 = Number(right?.y_max);
      if (
        !Number.isFinite(x1) ||
        !Number.isFinite(x2) ||
        !Number.isFinite(yMin1) ||
        !Number.isFinite(yMin2) ||
        !Number.isFinite(yMax1) ||
        !Number.isFinite(yMax2) ||
        yMax1 < yMin1 ||
        yMax2 < yMin2
      ) {
        continue;
      }
      segments.push([x1, yMin1, yMax1, x2, yMin2, yMax2]);
    }
    return segments;
  }, [points]);
  useEffect(() => {
    if (!isHistoryPanel || !tailPinned || typeof onValueChange !== "function" || !panelId) return;
    const normalized = normalizeZoomRange(zoomRange) || FULL_ZOOM;
    const width = Math.max(0, normalized.end - normalized.start);
    const next = { start: Math.max(0, 100 - width), end: 100 };
    if (!zoomRangeChanged(normalized, next)) return;
    onValueChange(panelId, writeZoomPanelValue(value, next, true), false);
  }, [isHistoryPanel, onValueChange, panelId, points.length, tailPinned, value, zoomRange]);
  const onDataZoom = useMemo(
    () => ({
      datazoom: (event) => {
        const next = readDataZoomRanges(event);
        if (!next || typeof onValueChange !== "function" || !panelId) return;
        const nextX = next.x || zoomRange;
        const nextY = next.y || yZoomRange;
        const xChanged = Boolean(next.x) && zoomRangeChanged(zoomRange, nextX);
        const yChanged = Boolean(next.y) && zoomRangeChanged(yZoomRange, nextY);
        if (!xChanged && !yChanged && (!isHistoryPanel || tailPinned === (nextX.end >= 99.5))) {
          return;
        }
        onValueChange(
          panelId,
          writeZoomPanelValue(value, nextX, isHistoryPanel ? nextX.end >= 99.5 : null, nextY),
          false,
        );
      },
    }),
    [isHistoryPanel, onValueChange, panelId, tailPinned, value, yZoomRange, zoomRange],
  );
  const option = useMemo(
    () => ({
      animation: false,
      legend: {
        top: 0,
        left: "center",
        textStyle: { color: "#475569", fontSize: 12 },
      },
      grid: { ...baseCartesianGrid, top: 52 },
      xAxis: {
        type: "value",
        min: xDomain[0],
        max: xDomain[1],
        name: usesTimestampXAxis ? "Elapsed Time" : inferNumericXAxisLabel(panelId, historyXAxisMode),
        axisLabel: {
          ...baseAxisLabel,
          formatter: (axisValue) =>
            formatTimeseriesXAxisValue(axisValue, historyXAxisMode, usesTimestampXAxis, xAxisOriginMs),
        },
        splitLine: { show: false },
        nameTextStyle: { color: "#64748b", fontSize: 12, padding: [12, 0, 0, 0] },
      },
      yAxis: {
        type: "value",
        min: visibleDomain[0],
        max: visibleDomain[1],
        axisLabel: baseAxisLabel,
        splitLine: { lineStyle: { color: gridColor } },
      },
      tooltip: {
        trigger: "axis",
        formatter: buildTimeseriesTooltipFormatter(historyXAxisMode, usesTimestampXAxis, xAxisOriginMs),
      },
      dataZoom: buildDataZoom(zoomRange, true, true, yZoomRange, true),
      series: [
        ...(isHistoryPanel
          ? [
              {
                name: "uncertainty",
                type: "custom",
                data: bandSegments,
                clip: true,
                silent: true,
                z: 1,
                tooltip: { show: false },
                renderItem: (params, api) => {
                  const x1 = Number(api.value(0));
                  const yMin1 = Number(api.value(1));
                  const yMax1 = Number(api.value(2));
                  const x2 = Number(api.value(3));
                  const yMin2 = Number(api.value(4));
                  const yMax2 = Number(api.value(5));
                  const p1 = api.coord([x1, yMin1]);
                  const p2 = api.coord([x1, yMax1]);
                  const p3 = api.coord([x2, yMax2]);
                  const p4 = api.coord([x2, yMin2]);
                  if ([p1, p2, p3, p4].some((point) => !Number.isFinite(point?.[0]) || !Number.isFinite(point?.[1]))) {
                    return null;
                  }
                  return {
                    type: "polygon",
                    shape: { points: [p1, p2, p3, p4] },
                    style: api.style({ fill: "rgba(124,138,150,0.22)", stroke: "none" }),
                  };
                },
              },
            ]
          : [buildErrorBarSeries({ name: "error", data: errorBarData })]),
        {
          type: "line",
          name: "y",
          data: meanData,
          smooth: Boolean(state?.smooth),
          showSymbol: false,
          lineStyle: { width: 1.8, color: "#005f73" },
          connectNulls: false,
        },
        ...(hasTargetLine
          ? [
              {
                type: "line",
                name: "target",
                data: [
                  [xDomain[0], targetValue],
                  [xDomain[1], targetValue],
                ],
                smooth: false,
                showSymbol: false,
                lineStyle: { width: 1.4, type: "dashed", color: "#ca6702" },
                connectNulls: false,
              },
            ]
          : []),
      ],
    }),
    [
      bandSegments,
      errorBarData,
      historyXAxisMode,
      isHistoryPanel,
      meanData,
      panelId,
      hasTargetLine,
      state?.smooth,
      targetValue,
      usesTimestampXAxis,
      visibleDomain,
      xAxisOriginMs,
      xDomain,
      yZoomRange,
      zoomRange,
    ],
  );
  if (points.length === 0) return null;
  return (
    <Card variant="outlined">
      <CardContent>
        <Box sx={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 2, mb: 2 }}>
          <Typography variant="subtitle1">{title}</Typography>
          <FigureExportActions
            baseName={state?.panel_id || title || "scalar_timeseries"}
            payload={{ panel_id: state?.panel_id ?? null, kind: "scalar_timeseries", state }}
            elementRef={figureRef}
            echartsRef={echartsRef}
            onResetView={
              panelId && typeof onValueChange === "function"
                ? () =>
                    onValueChange(
                      panelId,
                      writeZoomPanelValue(value, FULL_ZOOM, isHistoryPanel ? true : null, FULL_ZOOM),
                      false,
                    )
                : null
            }
          />
        </Box>
        <Box ref={figureRef} sx={{ width: "100%", height: 280 }}>
          <LazyChart
            ref={echartsRef}
            option={option}
            notMerge={false}
            onEvents={onDataZoom}
            lazyUpdate
            opts={{ renderer: "canvas" }}
            style={{ width: "100%", height: "100%" }}
          />
        </Box>
      </CardContent>
    </Card>
  );
};

const MultiTimeseriesPanel = ({ title, state, value = undefined, onValueChange = null }) => {
  const figureRef = useRef(null);
  const echartsRef = useRef(null);
  const panelId = state?.panel_id || null;
  const isHistoryPanel = useMemo(() => String(panelId || "").includes("_history"), [panelId]);
  const historyXAxisMode = readHistoryXAxisModeFromPanelValue(
    value,
    isHistoryPanel ? HISTORY_X_AXIS_MODE_SAMPLER_UPTIME : HISTORY_X_AXIS_MODE_WALL_TIME,
  );
  const series = asArray(state?.series).map((item) => ({
    ...item,
    points: remapAndSortTimeseriesPoints(item?.points, historyXAxisMode),
  }));
  const data = buildMultiSeriesData(series);
  const domain = fitDomain(
    data.flatMap((row) =>
      Object.entries(row)
        .filter(([key]) => key !== "x")
        .map(([, value]) => value),
    ),
  );
  const xDomain = fitXDomain(data.map((row) => row.x));
  const zoomRange = readZoomFromPanelValue(value, FULL_ZOOM);
  const yZoomRange = readYZoomFromPanelValue(value, FULL_ZOOM);
  const tailPinned = readTailPinnedFromPanelValue(value, isHistoryPanel);
  const usesTimestampXAxis = useMemo(
    () => historyXAxisMode === HISTORY_X_AXIS_MODE_WALL_TIME && isTimestampDomain(xDomain),
    [historyXAxisMode, xDomain],
  );
  const xAxisOriginMs = useMemo(() => (usesTimestampXAxis ? Number(xDomain[0]) : 0), [usesTimestampXAxis, xDomain]);
  useEffect(() => {
    if (!isHistoryPanel || !tailPinned || typeof onValueChange !== "function" || !panelId) return;
    const normalized = normalizeZoomRange(zoomRange) || FULL_ZOOM;
    const width = Math.max(0, normalized.end - normalized.start);
    const next = { start: Math.max(0, 100 - width), end: 100 };
    if (!zoomRangeChanged(normalized, next)) return;
    onValueChange(panelId, writeZoomPanelValue(value, next, true), false);
  }, [isHistoryPanel, onValueChange, panelId, data.length, tailPinned, value, zoomRange]);
  const onDataZoom = useMemo(
    () => ({
      datazoom: (event) => {
        const next = readDataZoomRanges(event);
        if (!next || typeof onValueChange !== "function" || !panelId) return;
        const nextX = next.x || zoomRange;
        const nextY = next.y || yZoomRange;
        const nextTailPinned = nextX.end >= 99.5;
        const xChanged = Boolean(next.x) && zoomRangeChanged(zoomRange, nextX);
        const yChanged = Boolean(next.y) && zoomRangeChanged(yZoomRange, nextY);
        if (!xChanged && !yChanged && (!isHistoryPanel || tailPinned === nextTailPinned)) return;
        onValueChange(
          panelId,
          writeZoomPanelValue(value, nextX, isHistoryPanel ? nextTailPinned : null, nextY),
          false,
        );
      },
    }),
    [isHistoryPanel, onValueChange, panelId, tailPinned, value, yZoomRange, zoomRange],
  );
  const option = useMemo(
    () => ({
      animation: false,
      legend: {
        top: 0,
        left: "center",
        textStyle: { color: "#475569", fontSize: 12 },
      },
      grid: { ...baseCartesianGrid, top: 52 },
      xAxis: {
        type: "value",
        min: xDomain[0],
        max: xDomain[1],
        name: usesTimestampXAxis ? "Elapsed Time" : inferNumericXAxisLabel(panelId, historyXAxisMode),
        axisLabel: {
          ...baseAxisLabel,
          formatter: (axisValue) =>
            formatTimeseriesXAxisValue(axisValue, historyXAxisMode, usesTimestampXAxis, xAxisOriginMs),
        },
        splitLine: { show: false },
        nameTextStyle: { color: "#64748b", fontSize: 12, padding: [12, 0, 0, 0] },
      },
      yAxis: {
        type: "value",
        min: domain[0],
        max: domain[1],
        axisLabel: baseAxisLabel,
        splitLine: { lineStyle: { color: gridColor } },
      },
      tooltip: {
        trigger: "axis",
        formatter: buildTimeseriesTooltipFormatter(historyXAxisMode, usesTimestampXAxis, xAxisOriginMs),
      },
      dataZoom: buildDataZoom(zoomRange, true, true, yZoomRange, true),
      series: series.map((item, index) => ({
        type: "line",
        name: item.label,
        data: asArray(item.points).map((point) => [Number(point?.x), Number(point?.y)]),
        smooth: Boolean(item?.smooth),
        showSymbol: false,
        connectNulls: false,
        lineStyle: {
          width: 1.8,
          color: item.color || lineColors[index % lineColors.length],
        },
        itemStyle: {
          color: item.color || lineColors[index % lineColors.length],
        },
      })),
    }),
    [domain, historyXAxisMode, panelId, series, usesTimestampXAxis, xAxisOriginMs, xDomain, yZoomRange, zoomRange],
  );
  if (data.length === 0) return null;
  return (
    <Card variant="outlined">
      <CardContent>
        <Box sx={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 2, mb: 2 }}>
          <Typography variant="subtitle1">{title}</Typography>
          <FigureExportActions
            baseName={state?.panel_id || title || "multi_timeseries"}
            payload={{ panel_id: state?.panel_id ?? null, kind: "multi_timeseries", state }}
            elementRef={figureRef}
            echartsRef={echartsRef}
            onResetView={
              panelId && typeof onValueChange === "function"
                ? () =>
                    onValueChange(
                      panelId,
                      writeZoomPanelValue(value, FULL_ZOOM, isHistoryPanel ? true : null, FULL_ZOOM),
                      false,
                    )
                : null
            }
          />
        </Box>
        <Box ref={figureRef} sx={{ width: "100%", height: 280 }}>
          <LazyChart
            ref={echartsRef}
            option={option}
            notMerge={false}
            onEvents={onDataZoom}
            lazyUpdate
            opts={{ renderer: "canvas" }}
            style={{ width: "100%", height: "100%" }}
          />
        </Box>
      </CardContent>
    </Card>
  );
};

const buildLinspaceParameters = (range, count) => {
  const [min, max] = asArray(range);
  if (!Number.isFinite(min) || !Number.isFinite(max) || count <= 0) {
    return Array.from({ length: count }, (_, index) => index);
  }
  if (count === 1) return [min];
  const step = (max - min) / (count - 1);
  return Array.from({ length: count }, (_, index) => min + step * index);
};

const buildScalarHeatmapScale = (values, normalizationMode, spread = 1) => {
  const finite = values.filter((value) => Number.isFinite(value));
  if (finite.length === 0) {
    return { zmin: 0, zmax: 1 };
  }
  if (normalizationMode === "symmetric") {
    const spreadFactor = clampHeatmapSpread(spread, 1);
    const maxAbs = Math.max(...finite.map((value) => Math.abs(value)), 1e-12) * spreadFactor;
    return { zmin: -maxAbs, zmax: maxAbs };
  }
  const zmin = Math.min(...finite);
  const zmax = Math.max(...finite);
  if (zmin === zmax) {
    const padding = Math.abs(zmin) > 0 ? Math.abs(zmin) * 0.1 : 1;
    return { zmin: zmin - padding, zmax: zmax + padding };
  }
  return { zmin, zmax };
};

const buildInvalidCellOverlay = (invalidIndices, width, height) => {
  const points = Array.from(invalidIndices || [])
    .map((index) => {
      const row = Math.floor(index / width);
      const col = index % width;
      if (row < 0 || row >= height || col < 0 || col >= width) return null;
      return [col, row];
    })
    .filter(Boolean);

  return points;
};

const ScalarImageHeatmapPanel = ({
  title,
  panelId = null,
  width,
  height,
  values,
  invalidIndices,
  normalizationMode,
  xRange,
  yRange,
  value = undefined,
  onValueChange = null,
}) => {
  const figureRef = useRef(null);
  const echartsRef = useRef(null);
  const [panelWidth, setPanelWidth] = useState(0);
  const xParameters = useMemo(() => buildLinspaceParameters(xRange, width), [width, xRange]);
  const yParameters = useMemo(() => buildLinspaceParameters(yRange, height), [height, yRange]);
  const totalCells = Math.max(0, width * height);
  const boundedValues = useMemo(() => values.slice(0, totalCells), [totalCells, values]);
  const isPdfPanel = typeof panelId === "string" && panelId.startsWith("pdf_adaptation_");
  const supportsSpreadControl = normalizationMode === "symmetric" && isPdfPanel && panelId && typeof onValueChange === "function";
  const spread = supportsSpreadControl ? readHeatmapSpreadFromPanelValue(value, 1) : 1;
  const { zmin, zmax } = useMemo(
    () => buildScalarHeatmapScale(boundedValues, normalizationMode, spread),
    [boundedValues, normalizationMode, spread],
  );
  const invalidOverlay = useMemo(
    () => buildInvalidCellOverlay(invalidIndices, width, height),
    [height, invalidIndices, width],
  );
  const heatmapData = useMemo(() => {
    const points = [];
    for (let row = 0; row < height; row += 1) {
      for (let col = 0; col < width; col += 1) {
        const index = row * width + col;
        if (index >= boundedValues.length) continue;
        if (invalidIndices?.has(index)) continue;
        const value = Number(boundedValues[index]);
        if (!Number.isFinite(value)) continue;
        points.push([col, row, value]);
      }
    }
    return points;
  }, [boundedValues, height, invalidIndices, width]);

  const heatmapMargins = useMemo(() => ({ left: 56, right: 154, top: 16, bottom: 44 }), []);
  const zoomRange = readZoomFromPanelValue(value, FULL_ZOOM);
  const yZoomRange = readYZoomFromPanelValue(value, FULL_ZOOM);
  const chartHeight = useMemo(() => {
    if (width <= 0 || height <= 0 || panelWidth <= 0) return 360;
    const innerWidth = Math.max(1, panelWidth - heatmapMargins.left - heatmapMargins.right);
    const innerHeight = (innerWidth * height) / width;
    return Math.max(220, Math.round(innerHeight + heatmapMargins.top + heatmapMargins.bottom));
  }, [heatmapMargins.bottom, heatmapMargins.left, heatmapMargins.right, heatmapMargins.top, height, panelWidth, width]);

  const option = useMemo(
    () => ({
      animation: false,
      grid: heatmapMargins,
      xAxis: {
        type: "category",
        data: Array.from({ length: width }, (_, index) => index),
        name: "t",
        axisLine: { show: true, lineStyle: { color: "#94a3b8" } },
        axisTick: { show: true },
        axisLabel: {
          color: "#64748b",
          fontSize: 11,
          formatter: (value) => {
            const index = Number(value);
            const parameter = Number.isFinite(index) ? xParameters[Math.max(0, Math.min(width - 1, index))] : Number.NaN;
            return Number.isFinite(parameter) ? formatScientific(parameter, 2) : "";
          },
          interval: Math.max(0, Math.ceil(width / 12) - 1),
        },
      },
      yAxis: {
        type: "category",
        data: Array.from({ length: height }, (_, index) => index),
        name: "s",
        axisLine: { show: true, lineStyle: { color: "#94a3b8" } },
        axisTick: { show: true },
        axisLabel: {
          color: "#64748b",
          fontSize: 11,
          formatter: (value) => {
            const index = Number(value);
            const parameter = Number.isFinite(index) ? yParameters[Math.max(0, Math.min(height - 1, index))] : Number.NaN;
            return Number.isFinite(parameter) ? formatScientific(parameter, 2) : "";
          },
          interval: Math.max(0, Math.ceil(height / 12) - 1),
        },
      },
      tooltip: {
        trigger: "item",
        formatter: (params) => {
          if (params?.seriesName === "invalid") return "invalid value";
          const data = Array.isArray(params?.data) ? params.data : [];
          const [col, row, value] = data;
          const x = Number.isFinite(Number(col)) ? xParameters[Math.max(0, Math.min(width - 1, Number(col)))] : Number.NaN;
          const y = Number.isFinite(Number(row))
            ? yParameters[Math.max(0, Math.min(height - 1, Number(row)))]
            : Number.NaN;
          return [
            `t: ${formatScientific(Number(x), 4)}`,
            `s: ${formatScientific(Number(y), 4)}`,
            `value: ${formatScientific(Number(value), 6)}`,
          ].join("<br/>");
        },
      },
      visualMap: {
        show: true,
        min: zmin,
        max: zmax,
        dimension: 2,
        orient: "vertical",
        right: 20,
        top: "middle",
        itemWidth: 22,
        itemHeight: 220,
        calculable: false,
        text: ["Value", ""],
        textStyle: { color: "#64748b", fontSize: 12 },
        inRange: { color: scalarHeatmapColors },
      },
      dataZoom: buildDataZoom(zoomRange, true, true, yZoomRange, true),
      series: [
        {
          type: "heatmap",
          name: "value",
          data: heatmapData,
          progressive: 0,
        },
        {
          type: "scatter",
          name: "invalid",
          data: invalidOverlay,
          symbol: "rect",
          symbolSize: 8,
          itemStyle: { color: "#ff00ff" },
          emphasis: { disabled: true },
        },
      ],
    }),
    [
      heatmapData,
      heatmapMargins,
      height,
      invalidOverlay,
      width,
      xParameters,
      zoomRange,
      yZoomRange,
      yParameters,
      zmax,
      zmin,
    ],
  );
  const onDataZoom = useMemo(
    () => ({
      datazoom: (event) => {
        if (typeof onValueChange !== "function" || !panelId) return;
        const next = readDataZoomRanges(event);
        const nextX = next?.x || zoomRange;
        const nextY = next?.y || yZoomRange;
        const xChanged = Boolean(next?.x) && zoomRangeChanged(zoomRange, nextX);
        const yChanged = Boolean(next?.y) && zoomRangeChanged(yZoomRange, nextY);
        if (!xChanged && !yChanged) return;
        onValueChange(panelId, writeZoomPanelValue(value, nextX, null, nextY), false);
      },
    }),
    [onValueChange, panelId, value, yZoomRange, zoomRange],
  );
  useEffect(() => {
    const element = figureRef.current;
    if (!element || typeof ResizeObserver === "undefined") return undefined;
    const updateWidth = () => {
      const measured = Math.max(0, Math.floor(element.getBoundingClientRect().width));
      setPanelWidth((current) => (current !== measured ? measured : current));
    };
    updateWidth();
    const rafId = requestAnimationFrame(updateWidth);
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (!entry) return;
      const nextWidth = Math.max(0, Math.floor(entry.contentRect.width));
      setPanelWidth((current) => (current !== nextWidth ? nextWidth : current));
    });
    observer.observe(element);
    return () => {
      cancelAnimationFrame(rafId);
      observer.disconnect();
    };
  }, []);
  return (
    <Card variant="outlined">
      <CardContent>
        <Box sx={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 2, mb: 2 }}>
          <Typography variant="subtitle1">{title}</Typography>
          <Stack direction="row" spacing={2} alignItems="center">
            {supportsSpreadControl ? (
              <Stack direction="row" spacing={1.25} alignItems="center" sx={{ minWidth: 240 }}>
                <Typography variant="caption" color="text.secondary" sx={{ whiteSpace: "nowrap" }}>
                  Spread
                </Typography>
                <Slider
                  size="small"
                  min={0.25}
                  max={4}
                  step={0.05}
                  value={spread}
                  onChangeCommitted={(_event, next) => {
                    const numeric = Array.isArray(next) ? next[0] : next;
                    onValueChange(panelId, writeHeatmapSpreadPanelValue(value, numeric), false);
                  }}
                  valueLabelDisplay="auto"
                  valueLabelFormat={(next) => `${Number(next).toFixed(2)}x`}
                  sx={{ width: 140 }}
                />
                <Button
                  size="small"
                  variant="text"
                  onClick={() => onValueChange(panelId, writeHeatmapSpreadPanelValue(value, 1), false)}
                  disabled={Math.abs(spread - 1) < 1e-9}
                >
                  Reset
                </Button>
              </Stack>
            ) : null}
            <FigureExportActions
              baseName={panelId || title || "image2d"}
              payload={{
                panel_id: panelId,
                kind: "image2d",
                state: {
                  width,
                  height,
                  x_range: xRange,
                  y_range: yRange,
                  values,
                  invalid_indices: Array.from(invalidIndices || []),
                },
              }}
              elementRef={figureRef}
              echartsRef={echartsRef}
              onResetView={
                panelId && typeof onValueChange === "function"
                  ? () => onValueChange(panelId, writeZoomPanelValue(value, FULL_ZOOM, null, FULL_ZOOM), false)
                  : null
              }
            />
          </Stack>
        </Box>
        <Box
          ref={figureRef}
          sx={{
            width: "min(100%, 920px)",
            mx: "auto",
            height: `${chartHeight}px`,
          }}
        >
          <LazyChart
            ref={echartsRef}
            option={option}
            notMerge={false}
            onEvents={onDataZoom}
            lazyUpdate
            opts={{ renderer: "canvas" }}
            style={{ width: "100%", height: "100%" }}
          />
        </Box>
      </CardContent>
    </Card>
  );
};

const HistogramPanel = ({
  title,
  state,
  value = undefined,
  onValueChange = null,
  uploadedBundles = [],
  onUpdateBundleSelection = null,
  onRemoveComparedHistogram = null,
  onAddComparedHistogram = null,
}) => {
  const figureRef = useRef(null);
  const echartsRef = useRef(null);
  const panelId = state?.panel_id || null;
  const isPdfPanel = typeof panelId === "string" && panelId.startsWith("pdf_adaptation_");
  const sourcePanelId = state?.source_panel_id || panelId;
  const isBundleControlled = sourcePanelId === "gammaloop_histogram_bundle";
  const currentHistogramName = typeof state?.name === "string" ? state.name : null;
  const defaultYScale = inferDefaultHistogramYScale(state);
  const defaultXScale = inferDefaultHistogramXScale(state);
  const [localYScale, setLocalYScale] = useState("linear");
  const [localXScale, setLocalXScale] = useState("linear");
  const [localSortMode, setLocalSortMode] = useState(HISTOGRAM_SORT_CANONICAL);
  const [localShowRelativeErrors, setLocalShowRelativeErrors] = useState(() => {
    const pid = state?.panel_id || "";
    return String(pid).startsWith("pdf_adaptation_") ? false : true;
  });
  const [localHistogramMode, setLocalHistogramMode] = useState(HISTOGRAM_MODE_PDF);
  const [localShowRatio, setLocalShowRatio] = useState(false);
  const yScale = isBundleControlled ? readHistogramScaleFromPanelValue(value, "y", defaultYScale) : localYScale;
  const xScale = isBundleControlled ? readHistogramScaleFromPanelValue(value, "x", defaultXScale) : localXScale;
  const zoomRange = isBundleControlled
    ? readHistogramZoomFromPanelValue(value, FULL_ZOOM)
    : readZoomFromPanelValue(value, FULL_ZOOM);
  const yZoomRange = isBundleControlled
    ? readHistogramYZoomFromPanelValue(value, FULL_ZOOM)
    : readYZoomFromPanelValue(value, FULL_ZOOM);
  const view = isBundleControlled ? readHistogramBundleView(value) : {};
  const showRelativeErrors = isBundleControlled ? view.show_relative_error !== false : localShowRelativeErrors;
  const histogramMode = isBundleControlled ? normalizeHistogramMode(view.display_mode) : localHistogramMode;
  const showRatio = isBundleControlled ? view.show_ratio === true : localShowRatio;
  const requestedSortMode = isBundleControlled
    ? normalizeHistogramSortMode(
        isObject(view?.sort_mode_by_histogram) && currentHistogramName
          ? view.sort_mode_by_histogram[currentHistogramName]
          : HISTOGRAM_SORT_CANONICAL,
      )
    : normalizeHistogramSortMode(localSortMode);
  const baseCanonicalBins = useMemo(() => buildHistogramData(state?.bins), [state?.bins]);
  const sortedBaseBins = useMemo(() => {
    const isDiscreteHistogram = baseCanonicalBins.some((bin) => bin && (bin.label != null || bin.bin_id != null));
    return isDiscreteHistogram
      ? sortHistogramBinsByMode(baseCanonicalBins, requestedSortMode)
      : baseCanonicalBins.slice().sort((left, right) => Number(left?.start) - Number(right?.start));
  }, [baseCanonicalBins, requestedSortMode]);
  const bins = useMemo(
    () => (histogramMode === HISTOGRAM_MODE_CDF ? buildCdfBins(sortedBaseBins) : sortedBaseBins),
    [histogramMode, sortedBaseBins],
  );
  const stepData = useMemo(() => buildHistogramRenderData(bins, yScale), [bins, yScale]);
  const relativeErrorData = useMemo(() => buildRelativeErrorStepData(bins), [bins]);
  // Detect discrete histograms: presence of bin labels/bin_id or explicit discrete ordering
  const isDiscrete = useMemo(() => {
    if (!Array.isArray(state?.bins)) return false;
    if (state?.discrete_ordering) return true;
    return state.bins.some((b) => b && (b.label != null || b.bin_id != null));
  }, [state]);

  const categories = useMemo(() => {
    if (!isDiscrete) return null;
    return bins.map((bin, idx) => {
      if (bin?.label != null) return String(bin.label);
      if (bin?.bin_id != null) return String(bin.bin_id);
      // fallback to range if available
      const start = Number(bin?.start);
      const stop = Number(bin?.stop);
      if (Number.isFinite(start) && Number.isFinite(stop))
        return `${formatScientific(start, 4)}→${formatScientific(stop, 4)}`;
      return `#${idx}`;
    });
  }, [isDiscrete, bins]);
  const discreteRelativeErrorData = useMemo(() => buildDiscreteRelativeErrorData(bins), [bins]);
  const hasPositiveContinuousEdges = useMemo(
    () =>
      asArray(bins).some((bin) => {
        const start = Number(bin?.start);
        const stop = Number(bin?.stop);
        return Number.isFinite(start) && Number.isFinite(stop) && (start > 0 || stop > 0);
      }),
    [bins],
  );
  const effectiveXScale =
    isDiscrete || xScale !== "log" || hasPositiveContinuousEdges ? (isDiscrete ? "linear" : xScale) : "linear";
  const discreteBaseKeys = useMemo(() => bins.map((bin, index) => discreteHistogramBinKey(bin, index)), [bins]);
  const discreteBaseSortedCanonicalIndices = useMemo(() => {
    if (!isDiscrete) return [];
    const canonicalIndexByBin = new Map(asArray(baseCanonicalBins).map((bin, index) => [bin, index]));
    return asArray(sortedBaseBins).map((bin) => {
      const index = canonicalIndexByBin.get(bin);
      return Number.isFinite(index) ? index : -1;
    });
  }, [baseCanonicalBins, sortedBaseBins, isDiscrete]);
  const comparedBundleSelections = useMemo(
    () =>
      asArray(uploadedBundles).map((bundle) => ({
        bundle,
        selectionState: normalizeHistogramSelectionState(bundle, currentHistogramName),
      })),
    [currentHistogramName, uploadedBundles],
  );
  const comparedOverlaySeries = useMemo(
    () =>
      comparedBundleSelections
        .flatMap(({ bundle, selectionState }, bundleIndex) => {
          const selectedNames = asArray(selectionState.selectedHistograms)
            .filter((name) => typeof name === "string")
            .filter((name, index, values) => values.indexOf(name) === index);
          return selectedNames.map((selectedName, selectedIndex) => {
            const histogram = bundle.histograms?.[selectedName];
            if (!histogram) return null;
            if (histogramIsDiscrete(histogram) !== isDiscrete) return null;
            const discreteMatchMode =
              selectionState.discreteAlignmentByHistogram?.[selectedName] === "by_index" ? "by_index" : "by_key";
            const color = histogramOverlayColors[(bundleIndex * 3 + selectedIndex) % histogramOverlayColors.length];
            const seriesLabel = `${bundle.label}: ${selectedName}`;
            if (isDiscrete) {
              const overlayCanonicalBins = buildHistogramData(histogram.bins);
              const overlayBinByKey = new Map(
                overlayCanonicalBins.map((bin, index) => [discreteHistogramBinKey(bin, index), bin]),
              );
              const matchedOverlayBins =
                discreteMatchMode === "by_index"
                  ? discreteBaseSortedCanonicalIndices.map((sourceIndex) => {
                      const bin = sourceIndex >= 0 ? overlayCanonicalBins[sourceIndex] : null;
                      return bin || null;
                    })
                  : (() => {
                      return discreteBaseKeys.map((key) => {
                        const bin = overlayBinByKey.get(key);
                        return bin || null;
                      });
                    })();
              const displayOverlayBins =
                histogramMode === HISTOGRAM_MODE_CDF
                  ? buildCdfBinsPreservingNulls(matchedOverlayBins)
                  : matchedOverlayBins;
              const values = displayOverlayBins.map((bin) => {
                if (!bin) return null;
                const numeric = Number(bin?.value);
                if (!Number.isFinite(numeric)) return null;
                return yScale === "log" ? signedLog10(numeric) : numeric;
              });
              const relative = displayOverlayBins.map((bin) => {
                if (!bin) return null;
                const value = Number(bin?.value);
                const error = Number(bin?.error);
                if (!Number.isFinite(value) || !Number.isFinite(error) || value === 0) return null;
                return Math.abs(error / value);
              });
              const absoluteError = values.map((value, index) => {
                if (!Number.isFinite(value)) return null;
                const sourceBin = displayOverlayBins[index];
                const err = Number(sourceBin?.error);
                if (!Number.isFinite(err) || err <= 0) return null;
                const sourceValue = Number(sourceBin?.value);
                if (!Number.isFinite(sourceValue)) return null;
                const yLowRaw = sourceValue - Math.abs(err);
                const yHighRaw = sourceValue + Math.abs(err);
                const yLow = yScale === "log" ? signedLog10(yLowRaw) : yLowRaw;
                const yHigh = yScale === "log" ? signedLog10(yHighRaw) : yHighRaw;
                return [index, yLow, yHigh];
              });
              return {
                id: `${bundle.id}-${selectedName}`,
                name: seriesLabel,
                color,
                discreteValues: values,
                discreteRelative: relative,
                discreteAbsError: absoluteError.filter(Boolean),
                ratioData: buildLogRatioPoints(bins, displayOverlayBins, true, effectiveXScale),
              };
            }
            const overlayCanonicalBins = buildHistogramData(histogram.bins).sort(
              (left, right) => Number(left?.start) - Number(right?.start),
            );
            const displayOverlayBins =
              histogramMode === HISTOGRAM_MODE_CDF ? buildCdfBins(overlayCanonicalBins) : overlayCanonicalBins;
            const projected = projectOverlayHistogramToReferenceBins(
              bins,
              displayOverlayBins,
              yScale,
              effectiveXScale,
            );
            return {
              id: `${bundle.id}-${selectedName}`,
              name: seriesLabel,
              color,
              valueStep: projected.valueStep,
              relativeStep: projected.relativeStep,
              absError: projected.absError,
              ratioData: buildLogRatioPoints(bins, projectBinsToReferenceBins(bins, displayOverlayBins), false, effectiveXScale),
            };
          });
        })
        .filter(Boolean),
    [
      bins,
      comparedBundleSelections,
      discreteBaseKeys,
      discreteBaseSortedCanonicalIndices,
      effectiveXScale,
      histogramMode,
      isDiscrete,
      yScale,
    ],
  );
  const embeddedOverlaySeries = useMemo(
    () =>
      asArray(state?.overlay_histograms)
        .map((overlay, overlayIndex) => {
          const overlayBins = asArray(overlay?.bins);
          if (overlayBins.length === 0) return null;
          const overlayName =
            typeof overlay?.name === "string" && overlay.name.trim().length > 0
              ? overlay.name
              : `overlay_${overlayIndex + 1}`;
          const overlayColor =
            typeof overlay?.color === "string" && overlay.color.trim().length > 0
              ? overlay.color
              : histogramOverlayColors[overlayIndex % histogramOverlayColors.length];

          if (isDiscrete) {
            const overlayCanonicalBins = buildHistogramData(overlayBins);
            const overlayBinByKey = new Map(
              overlayCanonicalBins.map((bin, index) => [discreteHistogramBinKey(bin, index), bin]),
            );
            const matchedOverlayBins = discreteBaseKeys.map((key) => overlayBinByKey.get(key) || null);
            const displayOverlayBins =
              histogramMode === HISTOGRAM_MODE_CDF
                ? buildCdfBinsPreservingNulls(matchedOverlayBins)
                : matchedOverlayBins;
            const values = displayOverlayBins.map((bin) => {
              if (!bin) return null;
              const numeric = Number(bin?.value);
              if (!Number.isFinite(numeric)) return null;
              return yScale === "log" ? signedLog10(numeric) : numeric;
            });
            const relative = displayOverlayBins.map((bin) => {
              if (!bin) return null;
              const value = Number(bin?.value);
              const error = Number(bin?.error);
              if (!Number.isFinite(value) || !Number.isFinite(error) || value === 0) return null;
              return Math.abs(error / value);
            });
            const absoluteError = values.map((value, index) => {
              if (!Number.isFinite(value)) return null;
              const sourceBin = displayOverlayBins[index];
              const err = Number(sourceBin?.error);
              if (!Number.isFinite(err) || err <= 0) return null;
              const sourceValue = Number(sourceBin?.value);
              if (!Number.isFinite(sourceValue)) return null;
              const yLowRaw = sourceValue - Math.abs(err);
              const yHighRaw = sourceValue + Math.abs(err);
              const yLow = yScale === "log" ? signedLog10(yLowRaw) : yLowRaw;
              const yHigh = yScale === "log" ? signedLog10(yHighRaw) : yHighRaw;
              return [index, yLow, yHigh];
            });
            return {
              id: `embedded-overlay-${overlayIndex}`,
              name: overlayName,
              color: overlayColor,
              discreteValues: values,
              discreteRelative: relative,
              discreteAbsError: absoluteError.filter(Boolean),
              ratioData: buildLogRatioPoints(bins, displayOverlayBins, true, effectiveXScale),
            };
          }
          const overlayCanonicalBins = buildHistogramData(overlayBins).sort(
            (left, right) => Number(left?.start) - Number(right?.start),
          );
          const displayOverlayBins =
            histogramMode === HISTOGRAM_MODE_CDF ? buildCdfBins(overlayCanonicalBins) : overlayCanonicalBins;
          const projected =
            isPdfPanel && binsShareEdges(bins, displayOverlayBins)
              ? buildOverlaySeriesFromBins(displayOverlayBins, yScale, effectiveXScale)
              : projectOverlayHistogramToReferenceBins(bins, displayOverlayBins, yScale, effectiveXScale);
          return {
            id: `embedded-overlay-${overlayIndex}`,
            name: overlayName,
            color: overlayColor,
            valueStep: projected.valueStep,
            relativeStep: projected.relativeStep,
            absError: projected.absError,
            ratioData: buildLogRatioPoints(
              bins,
              isPdfPanel && binsShareEdges(bins, displayOverlayBins)
                ? displayOverlayBins
                : projectBinsToReferenceBins(bins, displayOverlayBins),
              false,
              effectiveXScale,
            ),
          };
        })
        .filter(Boolean),
    [bins, discreteBaseKeys, effectiveXScale, histogramMode, isDiscrete, isPdfPanel, state?.overlay_histograms, yScale],
  );
  const overlaySeries = useMemo(
    () => [...comparedOverlaySeries, ...embeddedOverlaySeries],
    [comparedOverlaySeries, embeddedOverlaySeries],
  );

  const xDomain = useMemo(() => {
    if (isDiscrete) return null;
    if (effectiveXScale !== "log") return fitHistogramXDomain(bins);
    const positiveEdges = bins
      .flatMap((bin) => [Number(bin?.start), Number(bin?.stop)])
      .filter((value) => Number.isFinite(value) && value > 0);
    if (positiveEdges.length === 0) return [Number.EPSILON, 1];
    const min = Math.min(...positiveEdges);
    const max = Math.max(...positiveEdges);
    return [Math.max(min, Number.EPSILON), Math.max(max, min * (1 + Number.EPSILON))];
  }, [bins, effectiveXScale, isDiscrete]);
  const visibleXRange = useMemo(
    () => (isDiscrete ? null : visibleXRangeFromZoomWithScale(xDomain, zoomRange, effectiveXScale)),
    [xDomain, effectiveXScale, zoomRange, isDiscrete],
  );
  const yDomain = useMemo(() => {
    const baseDomain = buildHistogramYDomain(bins, yScale, visibleXRange);
    const extraValues = [];
    for (const overlay of asArray(overlaySeries)) {
      if (isDiscrete) {
        for (const y of asArray(overlay?.discreteValues)) {
          const numeric = Number(y);
          if (Number.isFinite(numeric)) extraValues.push(numeric);
        }
        for (const entry of asArray(overlay?.discreteAbsError)) {
          const low = Number(entry?.[1]);
          const high = Number(entry?.[2]);
          if (Number.isFinite(low)) extraValues.push(low);
          if (Number.isFinite(high)) extraValues.push(high);
        }
        continue;
      }
      for (const point of asArray(overlay?.valueStep)) {
        const x = Number(point?.[0]);
        const y = Number(point?.[1]);
        if (!Number.isFinite(x) || !Number.isFinite(y)) continue;
        if (visibleXRange && (x < visibleXRange.min || x > visibleXRange.max)) continue;
        extraValues.push(y);
      }
      for (const entry of asArray(overlay?.absError)) {
        const x = Number(entry?.[0]);
        const low = Number(entry?.[1]);
        const high = Number(entry?.[2]);
        if (!Number.isFinite(x)) continue;
        if (visibleXRange && (x < visibleXRange.min || x > visibleXRange.max)) continue;
        if (Number.isFinite(low)) extraValues.push(low);
        if (Number.isFinite(high)) extraValues.push(high);
      }
    }
    if (extraValues.length === 0) return baseDomain;
    const baseMin = Number(baseDomain?.[0]);
    const baseMax = Number(baseDomain?.[1]);
    const candidates = [
      ...extraValues,
      ...(Number.isFinite(baseMin) ? [baseMin] : []),
      ...(Number.isFinite(baseMax) ? [baseMax] : []),
    ].filter((value) => Number.isFinite(value));
    return fitDomain(candidates);
  }, [bins, isDiscrete, overlaySeries, visibleXRange, yScale]);
  const relativeErrorYDomain = useMemo(
    () => buildRelativeErrorYDomain(isDiscrete ? discreteRelativeErrorData : relativeErrorData, visibleXRange),
    [discreteRelativeErrorData, isDiscrete, relativeErrorData, visibleXRange],
  );
  const binErrorData = useMemo(
    () =>
      bins
        .map((bin, index) => {
          const x = isDiscrete ? null : Number(bin?.x);
          const y = Number(bin?.value);
          const err = Number(bin?.error);
          if (isDiscrete) {
            if (!Number.isFinite(y) || !Number.isFinite(err) || err <= 0) return null;
            const yLowRaw = y - Math.abs(err);
            const yHighRaw = y + Math.abs(err);
            const yLow = yScale === "log" ? signedLog10(yLowRaw) : yLowRaw;
            const yHigh = yScale === "log" ? signedLog10(yHighRaw) : yHighRaw;
            return [index, yLow, yHigh];
          }
          if (!Number.isFinite(x) || !Number.isFinite(y) || !Number.isFinite(err) || err <= 0) {
            return null;
          }
          const yLowRaw = y - Math.abs(err);
          const yHighRaw = y + Math.abs(err);
          const yLow = yScale === "log" ? signedLog10(yLowRaw) : yLowRaw;
          const yHigh = yScale === "log" ? signedLog10(yHighRaw) : yHighRaw;
          if (effectiveXScale === "log" && x <= 0) return null;
          return [x, yLow, yHigh];
        })
        .filter(Boolean),
    [bins, effectiveXScale, yScale, isDiscrete],
  );
  const histogramOption = useMemo(() => {
    const suppressOverlayErrorBars =
      panelId === "pdf_adaptation_integrand_pdf_histogram_overlay" ||
      panelId === "pdf_adaptation_oversampling_histogram_overlay";
    const legendEntries = ["value", ...overlaySeries.map((overlay) => overlay.name)];
    if (isDiscrete) {
      const categoriesData = categories || bins.map((_, idx) => `#${idx}`);
      const barData = bins.map((bin) => {
        const numericValue = Number(bin?.value);
        if (!Number.isFinite(numericValue)) return null;
        return {
          value: yScale === "log" ? signedLog10(numericValue) : numericValue,
          rawValue: numericValue,
        };
      });
      const discreteBarSeriesCount = 1 + overlaySeries.length;
      return {
        animation: false,
        legend: {
          show: true,
          top: 0,
          data: legendEntries,
          textStyle: { color: "#64748b", fontSize: 12 },
        },
        grid: baseCartesianGrid,
        xAxis: {
          type: "category",
          data: categoriesData,
          name: inferNumericXAxisLabel(panelId),
          axisLabel: {
            color: baseAxisLabel.color,
            fontSize: baseAxisLabel.fontSize,
            formatter: (axisValue) => formatCategoryAxisValue(axisValue),
            interval: 0,
            rotate: categoriesData.length > 6 ? 30 : 0,
          },
          splitLine: { show: false },
          nameTextStyle: { color: "#64748b", fontSize: 12, padding: [12, 0, 0, 0] },
        },
        yAxis: {
          type: "value",
          min: yDomain[0],
          max: yDomain[1],
          axisLabel: yScale === "log" ? { ...baseAxisLabel, formatter: formatSignedLogAxisValue } : baseAxisLabel,
          splitLine: { lineStyle: { color: gridColor } },
        },
        tooltip: {
          trigger: "item",
          formatter: (params) => {
            const idx = params.dataIndex;
            const label = categoriesData[idx] ?? String(idx);
            const val = Number(bins[idx]?.value);
            const err = Number(bins[idx]?.error);
            const absErrorText =
              Number.isFinite(err) && err > 0 ? `±${formatScientific(err, 6)}` : "n/a";
            const relError =
              Number.isFinite(val) && Number.isFinite(err) && val !== 0
                ? Math.abs(err / val)
                : null;
            const relErrorText = Number.isFinite(relError)
              ? formatScientific(relError, 6)
              : "n/a";
            return [
              `${escapeXml(label)}: ${formatScientific(val, 6)}`,
              `abs error: ${absErrorText}`,
              `rel error: ${relErrorText}`,
            ].join("<br/>");
          },
        },
        dataZoom: buildDataZoom(zoomRange, false, true, yZoomRange, true),
        series: [
          ...(Array.isArray(binErrorData) && binErrorData.length > 0
            ? [
                buildDiscreteOffsetErrorBarSeries({
                  name: "error",
                  data: binErrorData,
                  slotIndex: 0,
                  slotCount: discreteBarSeriesCount,
                }),
              ]
            : []),
          {
            type: "bar",
            name: "value",
            data: barData,
            barCategoryGap: DISCRETE_BAR_CATEGORY_GAP,
            barGap: DISCRETE_BAR_GAP,
            itemStyle:
              yScale === "log"
                ? {
                    color: (params) => histogramSignColorFromRaw(params?.data?.rawValue),
                  }
                : { color: "#005f73" },
          },
          ...overlaySeries.flatMap((overlay, index) => {
            const slotIndex = index + 1;
            const series = [
              {
                type: "bar",
                name: overlay.name,
                data: overlay.discreteValues,
                barCategoryGap: DISCRETE_BAR_CATEGORY_GAP,
                barGap: DISCRETE_BAR_GAP,
                itemStyle: { color: overlay.color, opacity: 0.52 },
                emphasis: { focus: "series" },
              },
            ];
            if (
              !suppressOverlayErrorBars &&
              Array.isArray(overlay.discreteAbsError) &&
              overlay.discreteAbsError.length > 0
            ) {
              series.push(
                buildDiscreteOffsetErrorBarSeries({
                  name: `${overlay.name} error`,
                  data: overlay.discreteAbsError,
                  color: overlay.color,
                  slotIndex,
                  slotCount: discreteBarSeriesCount,
                }),
              );
            }
            return series;
          }),
        ],
      };
    }

    const stepSeriesPoints = stepData
      .map((point) => ({
        x: Number(point?.x),
        y: Number(point?.y),
        rawY: Number(point?.raw_y ?? point?.y),
      }))
      .filter(
        (point) => Number.isFinite(point.x) && Number.isFinite(point.y) && (effectiveXScale !== "log" || point.x > 0),
      );
    const valueSeriesData = stepSeriesPoints.map((point) => [point.x, point.y]);
    const positiveSeriesData =
      yScale === "log"
        ? stepSeriesPoints.map((point) => (point.rawY > 0 ? [point.x, point.y] : [point.x, null]))
        : valueSeriesData;
    const negativeSeriesData =
      yScale === "log" ? stepSeriesPoints.map((point) => (point.rawY < 0 ? [point.x, point.y] : [point.x, null])) : [];
    const zeroSeriesData =
      yScale === "log"
        ? stepSeriesPoints.map((point) => (point.rawY === 0 ? [point.x, point.y] : [point.x, null]))
        : [];
    const valueSeries =
      yScale === "log"
        ? [
            {
              id: "histogram-value-positive",
              type: "line",
              name: "value (+)",
              data: positiveSeriesData,
              step: "end",
              showSymbol: false,
              lineStyle: { width: 1.35, color: HISTOGRAM_POSITIVE_COLOR },
              itemStyle: { color: HISTOGRAM_POSITIVE_COLOR },
              connectNulls: false,
              emphasis: { disabled: true },
              tooltip: { show: false },
            },
            {
              id: "histogram-value-negative",
              type: "line",
              name: "value (-)",
              data: negativeSeriesData,
              step: "end",
              showSymbol: false,
              lineStyle: { width: 1.35, color: HISTOGRAM_NEGATIVE_COLOR },
              itemStyle: { color: HISTOGRAM_NEGATIVE_COLOR },
              connectNulls: false,
              emphasis: { disabled: true },
              tooltip: { show: false },
            },
            {
              id: "histogram-value-zero",
              type: "line",
              name: "value (0)",
              data: zeroSeriesData,
              step: "end",
              showSymbol: false,
              lineStyle: { width: 1.35, color: HISTOGRAM_ZERO_COLOR },
              itemStyle: { color: HISTOGRAM_ZERO_COLOR },
              connectNulls: false,
              emphasis: { disabled: true },
              tooltip: { show: false },
            },
          ]
        : [
            {
              id: "histogram-value",
              type: "line",
              name: "value",
              data: valueSeriesData,
              step: "end",
              showSymbol: false,
              lineStyle: { width: 1.35, color: "#005f73" },
              connectNulls: false,
              emphasis: { disabled: true },
              tooltip: { show: false },
            },
          ];
    return {
      animation: false,
      legend: {
        show: true,
        top: 0,
        data: legendEntries,
        textStyle: { color: "#64748b", fontSize: 12 },
      },
      grid: baseCartesianGrid,
      xAxis: {
        type: effectiveXScale === "log" ? "log" : "value",
        min: xDomain[0],
        max: xDomain[1],
        name: inferNumericXAxisLabel(panelId),
        axisLabel: baseAxisLabel,
        splitLine: { show: false },
        nameTextStyle: { color: "#64748b", fontSize: 12, padding: [12, 0, 0, 0] },
      },
      yAxis: {
        type: "value",
        min: yDomain[0],
        max: yDomain[1],
        axisLabel: yScale === "log" ? { ...baseAxisLabel, formatter: formatSignedLogAxisValue } : baseAxisLabel,
        splitLine: { lineStyle: { color: gridColor } },
      },
      tooltip: {
        trigger: "axis",
        formatter: (params) => {
          const entries = asArray(params);
          const xValue = Number(entries[0]?.axisValue);
          if (!Number.isFinite(xValue)) return "n/a";
          const bin = bins.find((candidate) => {
            const start = Number(candidate?.start);
            const stop = Number(candidate?.stop);
            if (!Number.isFinite(start) || !Number.isFinite(stop)) return false;
            const inclusiveStop = Math.abs(xValue - stop) <= 1e-12;
            return xValue >= start && (xValue < stop || inclusiveStop);
          });
          if (!bin) {
            return `${formatScientific(xValue, 6)}: n/a`;
          }
          const valueNumeric = Number(bin?.value);
          const errorNumeric = Number(bin?.error);
          const absErrorText =
            Number.isFinite(errorNumeric) && errorNumeric > 0 ? `±${formatScientific(errorNumeric, 6)}` : "n/a";
          const relError =
            Number.isFinite(valueNumeric) && Number.isFinite(errorNumeric) && valueNumeric !== 0
              ? Math.abs(errorNumeric / valueNumeric)
              : null;
          const relErrorText = Number.isFinite(relError) ? formatScientific(relError, 6) : "n/a";
          return [
            `${escapeXml(`${formatScientific(Number(bin?.start), 6)} → ${formatScientific(Number(bin?.stop), 6)}`)}: ${formatScientific(valueNumeric, 6)}`,
            `abs error: ${absErrorText}`,
            `rel error: ${relErrorText}`,
          ].join("<br/>");
        },
      },
      dataZoom: buildDataZoom(zoomRange, false, true, yZoomRange, true),
      series: [
        ...(showRelativeErrors && Array.isArray(binErrorData) && binErrorData.length > 0
          ? [buildErrorBarSeries({ name: "error", data: binErrorData })]
          : []),
        ...valueSeries,
        ...overlaySeries.flatMap((overlay) => [
          ...(!suppressOverlayErrorBars && Array.isArray(overlay.absError) && overlay.absError.length > 0
            ? [buildErrorBarSeries({ name: `${overlay.name} error`, data: overlay.absError, color: overlay.color })]
            : []),
          {
            id: `histogram-overlay-${overlay.id}`,
            type: "line",
            name: overlay.name,
            data: overlay.valueStep,
            step: "end",
            showSymbol: false,
            connectNulls: false,
            lineStyle: { width: 1.4, type: "dashed", color: overlay.color },
            itemStyle: { color: overlay.color },
            emphasis: { disabled: true },
            tooltip: { show: false },
          },
        ]),
      ],
    };
  }, [
    binErrorData,
    bins,
    categories,
    effectiveXScale,
    isDiscrete,
    overlaySeries,
    panelId,
    showRelativeErrors,
    stepData,
    xDomain,
    yDomain,
    yScale,
    yZoomRange,
    zoomRange,
  ]);

  const relativeOption = useMemo(() => {
    if (isDiscrete) {
      const categoriesData = categories || bins.map((_, idx) => `#${idx}`);
      const discreteBarSeriesCount = 1 + overlaySeries.length;
      return {
        animation: false,
        grid: baseCartesianGrid,
        xAxis: {
          type: "category",
          data: categoriesData,
          name: inferNumericXAxisLabel(panelId),
          axisLabel: {
            color: baseAxisLabel.color,
            fontSize: baseAxisLabel.fontSize,
            formatter: (axisValue) => formatCategoryAxisValue(axisValue),
            interval: 0,
            rotate: categoriesData.length > 6 ? 30 : 0,
          },
          splitLine: { show: false },
          nameTextStyle: { color: "#64748b", fontSize: 12, padding: [12, 0, 0, 0] },
        },
        yAxis: {
          type: "value",
          min: relativeErrorYDomain[0],
          max: relativeErrorYDomain[1],
          axisLabel: baseAxisLabel,
          splitLine: { lineStyle: { color: gridColor } },
        },
        tooltip: {
          trigger: "axis",
          valueFormatter: (value) => (Number.isFinite(Number(value)) ? formatScientific(Number(value), 6) : "n/a"),
        },
        dataZoom: buildDataZoom(zoomRange, true, true, yZoomRange, true),
        series: [
          buildDiscreteOffsetRangeBarSeries({
            name: "relative_error",
            data: discreteRelativeErrorData
              .map((point) => {
                const rel = Number(point?.relative_error);
                if (!Number.isFinite(rel)) return null;
                return [Number(point?.index), -Math.abs(rel), Math.abs(rel)];
              })
              .filter(Boolean),
            color: "rgba(187, 62, 3, 0.55)",
            slotIndex: 0,
            slotCount: discreteBarSeriesCount,
          }),
          ...overlaySeries.map((overlay, index) =>
            buildDiscreteOffsetRangeBarSeries({
              name: overlay.name,
              data: overlay.discreteRelative
                .map((rel, relIndex) => {
                  const numeric = Number(rel);
                  if (!Number.isFinite(numeric)) return null;
                  return [relIndex, -Math.abs(numeric), Math.abs(numeric)];
                })
                .filter(Boolean),
              color: overlay.color,
              slotIndex: index + 1,
              slotCount: discreteBarSeriesCount,
            }),
          ),
        ],
      };
    }
    if (!xDomain) {
      return null;
    }
    return {
      animation: false,
      grid: baseCartesianGrid,
      xAxis: {
        type: effectiveXScale === "log" ? "log" : "value",
        min: xDomain[0],
        max: xDomain[1],
        name: inferNumericXAxisLabel(panelId),
        axisLabel: baseAxisLabel,
        splitLine: { show: false },
        nameTextStyle: { color: "#64748b", fontSize: 12, padding: [12, 0, 0, 0] },
      },
      yAxis: {
        type: "value",
        min: relativeErrorYDomain[0],
        max: relativeErrorYDomain[1],
        axisLabel: baseAxisLabel,
        splitLine: { lineStyle: { color: gridColor } },
      },
      tooltip: {
        trigger: "axis",
        valueFormatter: (value) => (Number.isFinite(Number(value)) ? formatScientific(Number(value), 6) : "n/a"),
      },
      dataZoom: buildDataZoom(zoomRange, true, true, yZoomRange, true),
      series: [
        {
          type: "line",
          name: "positive_relative_error",
          data: relativeErrorData
            .map((point) => [Number(point?.x), Number(point?.positive_relative_error)])
            .filter(([x]) => Number.isFinite(x) && (effectiveXScale !== "log" || x > 0)),
          step: "end",
          showSymbol: false,
          lineStyle: { width: 1.2, color: "#bb3e03" },
          areaStyle: { color: "rgba(187, 62, 3, 0.22)" },
          connectNulls: false,
        },
        {
          type: "line",
          name: "negative_relative_error",
          data: relativeErrorData
            .map((point) => [Number(point?.x), Number(point?.negative_relative_error)])
            .filter(([x]) => Number.isFinite(x) && (effectiveXScale !== "log" || x > 0)),
          step: "end",
          showSymbol: false,
          lineStyle: { width: 1.2, color: "#bb3e03" },
          areaStyle: { color: "rgba(187, 62, 3, 0.22)" },
          connectNulls: false,
        },
        ...overlaySeries.map((overlay) => ({
          type: "line",
          name: overlay.name,
          data: overlay.relativeStep,
          step: "end",
          showSymbol: false,
          connectNulls: false,
          lineStyle: { width: 1.35, type: "dashed", color: overlay.color },
          itemStyle: { color: overlay.color },
          emphasis: { focus: "series" },
        })),
      ],
    };
  }, [
    bins,
    categories,
    discreteRelativeErrorData,
    effectiveXScale,
    isDiscrete,
    overlaySeries,
    panelId,
    relativeErrorData,
    relativeErrorYDomain,
    xDomain,
    yZoomRange,
    zoomRange,
  ]);

  const ratioOption = useMemo(() => {
    const ratioSeries = overlaySeries
      .map((overlay) => ({
        ...overlay,
        ratioData: asArray(overlay?.ratioData).filter((point) => point && point.every((value) => Number.isFinite(Number(value)))),
      }))
      .filter((overlay) => overlay.ratioData.length > 0);
    if (ratioSeries.length === 0) return null;
    const ratioValues = ratioSeries.flatMap((overlay) =>
      overlay.ratioData.flatMap((point) => [Number(point?.[1]), Number(point?.[2]), Number(point?.[3])]),
    );
    const ratioYDomain = fitDomain([...ratioValues, 0]);
    if (isDiscrete) {
      const categoriesData = categories || bins.map((_, idx) => `#${idx}`);
      return {
        animation: false,
        legend: {
          show: true,
          top: 0,
          data: ratioSeries.map((overlay) => overlay.name),
          textStyle: { color: "#64748b", fontSize: 12 },
        },
        grid: baseCartesianGrid,
        xAxis: {
          type: "category",
          data: categoriesData,
          name: inferNumericXAxisLabel(panelId),
          axisLabel: {
            color: baseAxisLabel.color,
            fontSize: baseAxisLabel.fontSize,
            formatter: (axisValue) => formatCategoryAxisValue(axisValue),
            interval: 0,
            rotate: categoriesData.length > 6 ? 30 : 0,
          },
          splitLine: { show: false },
          nameTextStyle: { color: "#64748b", fontSize: 12, padding: [12, 0, 0, 0] },
        },
        yAxis: {
          type: "value",
          min: ratioYDomain[0],
          max: ratioYDomain[1],
          name: "log10(value / comparison)",
          axisLabel: baseAxisLabel,
          splitLine: { lineStyle: { color: gridColor } },
        },
        tooltip: {
          trigger: "axis",
          valueFormatter: (value) => (Number.isFinite(Number(value)) ? formatScientific(Number(value), 6) : "n/a"),
        },
        dataZoom: buildDataZoom(zoomRange, true, true, yZoomRange, true),
        series: ratioSeries.flatMap((overlay, index) => [
          buildDiscreteOffsetErrorBarSeries({
            name: `${overlay.name} ratio error`,
            data: overlay.ratioData.map((point) => [point[0], point[2], point[3]]),
            color: overlay.color,
            slotIndex: index,
            slotCount: ratioSeries.length,
          }),
          {
            type: "line",
            name: overlay.name,
            data: overlay.ratioData.map((point) => [point[0], point[1]]),
            showSymbol: true,
            symbolSize: 4,
            lineStyle: { width: 1.35, color: overlay.color },
            itemStyle: { color: overlay.color },
            markLine: {
              silent: true,
              symbol: "none",
              data: [{ yAxis: 0 }],
              lineStyle: { color: "#94a3b8", type: "dotted", width: 1 },
              label: { show: false },
            },
          },
        ]),
      };
    }
    if (!xDomain) return null;
    return {
      animation: false,
      legend: {
        show: true,
        top: 0,
        data: ratioSeries.map((overlay) => overlay.name),
        textStyle: { color: "#64748b", fontSize: 12 },
      },
      grid: baseCartesianGrid,
      xAxis: {
        type: effectiveXScale === "log" ? "log" : "value",
        min: xDomain[0],
        max: xDomain[1],
        name: inferNumericXAxisLabel(panelId),
        axisLabel: baseAxisLabel,
        splitLine: { show: false },
        nameTextStyle: { color: "#64748b", fontSize: 12, padding: [12, 0, 0, 0] },
      },
      yAxis: {
        type: "value",
        min: ratioYDomain[0],
        max: ratioYDomain[1],
        name: "log10(value / comparison)",
        axisLabel: baseAxisLabel,
        splitLine: { lineStyle: { color: gridColor } },
      },
      tooltip: {
        trigger: "axis",
        valueFormatter: (value) => (Number.isFinite(Number(value)) ? formatScientific(Number(value), 6) : "n/a"),
      },
      dataZoom: buildDataZoom(zoomRange, true, true, yZoomRange, true),
      series: ratioSeries.flatMap((overlay) => [
        buildErrorBarSeries({
          name: `${overlay.name} ratio error`,
          data: overlay.ratioData.map((point) => [point[0], point[2], point[3]]),
          color: overlay.color,
        }),
        {
          type: "line",
          name: overlay.name,
          data: overlay.ratioData.map((point) => [point[0], point[1]]),
          showSymbol: false,
          connectNulls: false,
          lineStyle: { width: 1.35, color: overlay.color },
          itemStyle: { color: overlay.color },
          markLine: {
            silent: true,
            symbol: "none",
            data: [{ yAxis: 0 }],
            lineStyle: { color: "#94a3b8", type: "dotted", width: 1 },
            label: { show: false },
          },
        },
      ]),
    };
  }, [bins, categories, effectiveXScale, isDiscrete, overlaySeries, panelId, xDomain, yZoomRange, zoomRange]);

  const onDataZoom = useMemo(
    () => ({
      datazoom: (event) => {
        const next = readDataZoomRanges(event);
        if (!next || typeof onValueChange !== "function" || !sourcePanelId) return;
        const nextX = next.x || zoomRange;
        const nextY = next.y || yZoomRange;
        const xChanged = Boolean(next.x) && zoomRangeChanged(zoomRange, nextX);
        const yChanged = Boolean(next.y) && zoomRangeChanged(yZoomRange, nextY);
        if (!xChanged && !yChanged) return;
        onValueChange(
          sourcePanelId,
          isBundleControlled
            ? writeHistogramBundlePanelValue(value, { zoom: nextX, yZoom: nextY })
            : writeZoomPanelValue(value, nextX, null, nextY),
          false,
        );
      },
    }),
    [isBundleControlled, onValueChange, sourcePanelId, value, yZoomRange, zoomRange],
  );

  if (bins.length === 0) return null;
  return (
    <Card variant="outlined">
      <CardContent>
        <Box sx={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 2, mb: 2 }}>
          <Typography variant="subtitle1">
            {title}
            {state?.name ? `  (${state.name})` : ""}
          </Typography>
          <Stack direction="row" spacing={1} alignItems="center">
            <FigureExportActions
              baseName={state?.panel_id || state?.name || title || "histogram"}
              payload={{ panel_id: state?.panel_id ?? null, kind: "histogram", state, xScale: effectiveXScale, yScale }}
              elementRef={figureRef}
              onResetView={
                sourcePanelId && typeof onValueChange === "function"
                  ? () =>
                      onValueChange(
                        sourcePanelId,
                        isBundleControlled
                          ? writeHistogramBundlePanelValue(value, { zoom: FULL_ZOOM, yZoom: FULL_ZOOM })
                          : writeZoomPanelValue(value, FULL_ZOOM, null, FULL_ZOOM),
                        false,
                      )
                  : null
              }
            />
            <FormControl size="small" sx={{ minWidth: 128 }}>
              <Select
                value={yScale}
                onChange={(event) => {
                  const next = event.target.value;
                  if (isBundleControlled && sourcePanelId && typeof onValueChange === "function") {
                    onValueChange(sourcePanelId, writeHistogramBundlePanelValue(value, { yScale: next }), false);
                    return;
                  }
                  setLocalYScale(next);
                }}
                sx={{
                  fontSize: "0.875rem",
                  ".MuiSelect-select": { py: 0.75 },
                }}
              >
                <MenuItem value="linear">Y Linear</MenuItem>
                <MenuItem value="log">Y Log</MenuItem>
              </Select>
            </FormControl>
            <FormControl size="small" sx={{ minWidth: 128 }}>
              <Select
                value={effectiveXScale}
                disabled={isDiscrete}
                onChange={(event) => {
                  const next = event.target.value;
                  if (isDiscrete) {
                    return;
                  }
                  if (isBundleControlled && sourcePanelId && typeof onValueChange === "function") {
                    onValueChange(sourcePanelId, writeHistogramBundlePanelValue(value, { xScale: next }), false);
                    return;
                  }
                  setLocalXScale(next);
                }}
                sx={{
                  fontSize: "0.875rem",
                  ".MuiSelect-select": { py: 0.75 },
                }}
              >
                <MenuItem value="linear">X Linear</MenuItem>
                <MenuItem value="log">X Log</MenuItem>
              </Select>
            </FormControl>
            <Button
              size="small"
              variant={histogramMode === HISTOGRAM_MODE_CDF ? "contained" : "outlined"}
              onClick={() => {
                const next = histogramMode === HISTOGRAM_MODE_CDF ? HISTOGRAM_MODE_PDF : HISTOGRAM_MODE_CDF;
                if (isBundleControlled && sourcePanelId && typeof onValueChange === "function") {
                  onValueChange(sourcePanelId, writeHistogramBundlePanelValue(value, { histogramMode: next }), false);
                  return;
                }
                setLocalHistogramMode(next);
              }}
            >
              {histogramMode === HISTOGRAM_MODE_CDF ? "CDF" : "PDF"}
            </Button>
            <Button
              size="small"
              variant={showRatio ? "contained" : "outlined"}
              disabled={!ratioOption}
              onClick={() => {
                const next = !showRatio;
                if (isBundleControlled && sourcePanelId && typeof onValueChange === "function") {
                  onValueChange(sourcePanelId, writeHistogramBundlePanelValue(value, { showRatio: next }), false);
                  return;
                }
                setLocalShowRatio(next);
              }}
            >
              Ratio
            </Button>
            {isDiscrete ? (
              <FormControl size="small" sx={{ minWidth: 156 }}>
                <Select
                  value={requestedSortMode}
                  onChange={(event) => {
                    const next = normalizeHistogramSortMode(event.target.value);
                    if (
                      isBundleControlled &&
                      sourcePanelId &&
                      typeof onValueChange === "function" &&
                      currentHistogramName
                    ) {
                      onValueChange(
                        sourcePanelId,
                        writeHistogramBundlePanelValue(value, {
                          sortModeByHistogram: { [currentHistogramName]: next },
                        }),
                        false,
                      );
                      return;
                    }
                    setLocalSortMode(next);
                  }}
                  sx={{
                    fontSize: "0.875rem",
                    ".MuiSelect-select": { py: 0.75 },
                  }}
                >
                  <MenuItem value={HISTOGRAM_SORT_CANONICAL}>Default</MenuItem>
                  <MenuItem value={HISTOGRAM_SORT_BY_VALUE}>By Value</MenuItem>
                  <MenuItem value={HISTOGRAM_SORT_BY_ABS_VALUE}>By |Value|</MenuItem>
                </Select>
              </FormControl>
            ) : null}
            <FormControlLabel
              control={
                <Switch
                  size="small"
                  checked={Boolean(showRelativeErrors)}
                  onChange={(event) => {
                    const next = Boolean(event.target.checked);
                    if (isBundleControlled && sourcePanelId && typeof onValueChange === "function") {
                      onValueChange(
                        sourcePanelId,
                        writeHistogramBundlePanelValue(value, { showRelativeError: next }),
                        false,
                      );
                      return;
                    }
                    setLocalShowRelativeErrors(next);
                  }}
                />
              }
              label="Relative Error"
              sx={{ mr: 1 }}
            />
          </Stack>
        </Box>
        {comparedBundleSelections.length > 0 ? (
          <Box sx={{ mb: 1.5, display: "grid", gap: 1 }}>
            {comparedBundleSelections.map(({ bundle, selectionState }) => {
              const histogramNames = Object.keys(bundle.histograms || {}).filter(
                (name) => histogramIsDiscrete(bundle.histograms?.[name]) === isDiscrete,
              );
              const usedNames = new Set(
                asArray(selectionState.selectedHistograms).filter((value) => typeof value === "string"),
              );
              const hasAddableComparison = histogramNames.some((name) => !usedNames.has(name));
              return (
                <Box
                  key={bundle.id}
                  sx={{
                    display: "grid",
                    gap: 1,
                    border: "1px solid",
                    borderColor: "divider",
                    borderRadius: 1,
                    px: 1,
                    py: 0.75,
                  }}
                >
                  <Stack direction="row" spacing={1} alignItems="center" sx={{ flexWrap: "wrap", rowGap: 0.75 }}>
                    <Typography variant="caption" color="text.secondary" sx={{ minWidth: 0, mr: 0.5 }}>
                      {bundle.label}
                    </Typography>
                    {isDiscrete ? (
                      <Typography variant="caption" color="text.secondary">
                        Discrete matching is configured per histogram row.
                      </Typography>
                    ) : null}
                    <Button
                      size="small"
                      variant="outlined"
                      disabled={!hasAddableComparison}
                      onClick={() =>
                        onAddComparedHistogram?.(sourcePanelId, bundle.id, currentHistogramName, isDiscrete)
                      }
                    >
                      Add Histogram
                    </Button>
                  </Stack>
                  <Box sx={{ display: "grid", gap: 0.75 }}>
                    {selectionState.selectedHistograms.map((comparedName, comparedIndex) => (
                      <Box
                        key={`${bundle.id}-${comparedIndex}`}
                        sx={{
                          display: "grid",
                          gridTemplateColumns: {
                            xs: "1fr auto",
                            sm: isDiscrete ? "minmax(0, 200px) 150px 150px auto" : "minmax(0, 220px) auto",
                          },
                          gap: 0.75,
                          alignItems: "center",
                        }}
                      >
                        <FormControl size="small">
                          <Select
                            value={comparedName}
                            onChange={(event) => {
                              const nextSelectedHistograms = selectionState.selectedHistograms.slice();
                              nextSelectedHistograms[comparedIndex] = String(event.target.value || "");
                              const nextDiscreteAlignmentByHistogram = {
                                ...(selectionState.discreteAlignmentByHistogram || {}),
                              };
                              const nextSortModeByHistogram = {
                                ...(selectionState.sortModeByHistogram || {}),
                              };
                              const oldName = selectionState.selectedHistograms[comparedIndex];
                              const nextName = nextSelectedHistograms[comparedIndex];
                              if (oldName && oldName !== nextName && !nextSelectedHistograms.includes(oldName)) {
                                delete nextDiscreteAlignmentByHistogram[oldName];
                                delete nextSortModeByHistogram[oldName];
                              }
                              if (nextName && !nextDiscreteAlignmentByHistogram[nextName]) {
                                nextDiscreteAlignmentByHistogram[nextName] = "by_key";
                              }
                              if (nextName && !nextSortModeByHistogram[nextName]) {
                                nextSortModeByHistogram[nextName] = HISTOGRAM_SORT_CANONICAL;
                              }
                              onUpdateBundleSelection?.(
                                sourcePanelId,
                                bundle.id,
                                currentHistogramName,
                                nextSelectedHistograms,
                                nextDiscreteAlignmentByHistogram,
                                nextSortModeByHistogram,
                              );
                            }}
                            sx={{ minWidth: 140, fontSize: "0.8125rem", ".MuiSelect-select": { py: 0.625 } }}
                          >
                            {histogramNames.map((name) => (
                              <MenuItem key={`${bundle.id}-${comparedIndex}-${name}`} value={name}>
                                {name}
                              </MenuItem>
                            ))}
                          </Select>
                        </FormControl>
                        {isDiscrete ? (
                          <FormControl size="small">
                            <Select
                              value={normalizeHistogramSortMode(selectionState.sortModeByHistogram?.[comparedName])}
                              onChange={(event) => {
                                const nextSortModeByHistogram = {
                                  ...(selectionState.sortModeByHistogram || {}),
                                  [comparedName]: normalizeHistogramSortMode(event.target.value),
                                };
                                onUpdateBundleSelection?.(
                                  sourcePanelId,
                                  bundle.id,
                                  currentHistogramName,
                                  selectionState.selectedHistograms,
                                  selectionState.discreteAlignmentByHistogram,
                                  nextSortModeByHistogram,
                                );
                              }}
                              sx={{ minWidth: 140, fontSize: "0.8125rem", ".MuiSelect-select": { py: 0.625 } }}
                            >
                              <MenuItem value={HISTOGRAM_SORT_CANONICAL}>Canonical</MenuItem>
                              <MenuItem value={HISTOGRAM_SORT_BY_VALUE}>By Value</MenuItem>
                              <MenuItem value={HISTOGRAM_SORT_BY_ABS_VALUE}>By |Value|</MenuItem>
                            </Select>
                          </FormControl>
                        ) : null}
                        {isDiscrete ? (
                          <FormControl size="small">
                            <Select
                              value={
                                selectionState.discreteAlignmentByHistogram?.[comparedName] === "by_index"
                                  ? "by_index"
                                  : "by_key"
                              }
                              onChange={(event) => {
                                const nextDiscreteAlignmentByHistogram = {
                                  ...(selectionState.discreteAlignmentByHistogram || {}),
                                  [comparedName]: event.target.value === "by_index" ? "by_index" : "by_key",
                                };
                                onUpdateBundleSelection?.(
                                  sourcePanelId,
                                  bundle.id,
                                  currentHistogramName,
                                  selectionState.selectedHistograms,
                                  nextDiscreteAlignmentByHistogram,
                                  selectionState.sortModeByHistogram,
                                );
                              }}
                              sx={{ minWidth: 140, fontSize: "0.8125rem", ".MuiSelect-select": { py: 0.625 } }}
                            >
                              <MenuItem value="by_key">Match: Key</MenuItem>
                              <MenuItem value="by_index">Match: Index</MenuItem>
                            </Select>
                          </FormControl>
                        ) : null}
                        <Button
                          size="small"
                          variant="text"
                          color="error"
                          onClick={() =>
                            onRemoveComparedHistogram?.(sourcePanelId, bundle.id, currentHistogramName, comparedIndex)
                          }
                        >
                          Remove
                        </Button>
                      </Box>
                    ))}
                  </Box>
                </Box>
              );
            })}
          </Box>
        ) : null}
        <Box ref={figureRef} sx={{ width: "100%", display: "grid", gap: 2 }}>
          <Box sx={{ width: "100%", height: 280 }}>
            <LazyChart
              ref={echartsRef}
              option={histogramOption}
              notMerge={!isDiscrete}
              onEvents={onDataZoom}
              lazyUpdate
              opts={{ renderer: "canvas" }}
              style={{ width: "100%", height: "100%" }}
            />
          </Box>
          {showRelativeErrors && relativeOption ? (
            <Box>
              <Typography variant="caption" color="text.secondary">
                Relative Error Shape
              </Typography>
              <Box sx={{ width: "100%", height: 168 }}>
                <LazyChart
                  option={relativeOption}
                  notMerge
                  onEvents={onDataZoom}
                  lazyUpdate
                  opts={{ renderer: "canvas" }}
                  style={{ width: "100%", height: "100%" }}
                />
              </Box>
            </Box>
          ) : null}
          {showRatio && ratioOption ? (
            <Box>
              <Typography variant="caption" color="text.secondary">
                Histogram Ratio: log10(value / comparison), agreement at 0
              </Typography>
              <Box sx={{ width: "100%", height: 188 }}>
                <LazyChart
                  option={ratioOption}
                  notMerge
                  onEvents={onDataZoom}
                  lazyUpdate
                  opts={{ renderer: "canvas" }}
                  style={{ width: "100%", height: "100%" }}
                />
              </Box>
            </Box>
          ) : null}
        </Box>
      </CardContent>
    </Card>
  );
};

const Image2dPanel = ({ title, state, value = undefined, onValueChange = null }) => {
  const width = Number(state?.width) || 0;
  const height = Number(state?.height) || 0;
  const values = useMemo(() => asArray(state?.values), [state?.values]);
  const imagValues = useMemo(() => {
    const next = asArray(state?.imag_values);
    return next.length > 0 ? next : null;
  }, [state?.imag_values]);
  const invalidIndices = useMemo(() => new Set(asArray(state?.invalid_indices)), [state?.invalid_indices]);
  const normalizationMode = state?.normalization_mode || "min_max";
  const xRange = useMemo(() => asArray(state?.x_range), [state?.x_range]);
  const yRange = useMemo(() => asArray(state?.y_range), [state?.y_range]);
  const scalarValues = useMemo(() => {
    if (!imagValues) return values;
    return values.map((re, index) => {
      const im = imagValues[index] || 0;
      if (!Number.isFinite(re) || !Number.isFinite(im)) return Number.NaN;
      return Math.hypot(re, im);
    });
  }, [imagValues, values]);
  if (width <= 0 || height <= 0 || values.length === 0) return null;
  return (
    <ScalarImageHeatmapPanel
      title={title}
      panelId={state?.panel_id ?? null}
      width={width}
      height={height}
      values={scalarValues}
      invalidIndices={invalidIndices}
      normalizationMode={normalizationMode}
      xRange={xRange}
      yRange={yRange}
      value={value}
      onValueChange={onValueChange}
    />
  );
};

const PanelRenderer = ({
  descriptor,
  state,
  value,
  onValueChange,
  histogramBundlesByPanel,
  histogramBundleUploadErrors,
  onUploadHistogramBundle,
  onRemoveHistogramBundle,
  onUpdateHistogramBundleSelection,
  onRemoveComparedHistogram,
  onAddComparedHistogram,
}) => {
  if (!descriptor) return null;
  switch (descriptor.kind) {
    case "select":
      return (
        <SelectPanel title={descriptor.label} descriptor={descriptor} value={value} onValueChange={onValueChange} />
      );
    case "scalar_timeseries":
      if (!state) return null;
      return (
        <ScalarTimeseriesPanel
          title={descriptor.label}
          state={{ ...state, panel_id: descriptor.panel_id }}
          value={value}
          onValueChange={onValueChange}
        />
      );
    case "multi_timeseries":
      if (!state) return null;
      return (
        <MultiTimeseriesPanel
          title={descriptor.label}
          state={{ ...state, panel_id: descriptor.panel_id }}
          value={value}
          onValueChange={onValueChange}
        />
      );
    case "tick_breakdown":
      if (!state) return null;
      return <TickBreakdownPanel title={descriptor.label} state={{ ...state, panel_id: descriptor.panel_id }} />;
    case "progress":
      if (!state) return null;
      return <ProgressPanel title={descriptor.label} state={state} />;
    case "key_value":
      if (!state) return null;
      return <KeyValuePanel title={descriptor.label} state={state} />;
    case "image2d":
      if (!state) return null;
      return (
        <Image2dPanel
          title={descriptor.label}
          state={{ ...state, panel_id: descriptor.panel_id }}
          value={value}
          onValueChange={onValueChange}
        />
      );
    case "table":
      if (!state) return null;
      return (
        <TablePanel
          title={descriptor.label}
          state={{ ...state, panel_id: descriptor.panel_id, selected_value: value, onValueChange }}
          uploadedBundles={histogramBundlesByPanel?.[descriptor.panel_id] ?? []}
          bundleUploadError={histogramBundleUploadErrors?.[descriptor.panel_id] ?? null}
          onUploadBundle={onUploadHistogramBundle}
          onRemoveBundle={onRemoveHistogramBundle}
        />
      );
    case "histogram":
      if (!state) return null;
      return (
        <HistogramPanel
          title={descriptor.label}
          state={{ ...state, panel_id: descriptor.panel_id }}
          value={value}
          onValueChange={onValueChange}
          uploadedBundles={histogramBundlesByPanel?.[state?.source_panel_id || descriptor.panel_id] ?? []}
          onUpdateBundleSelection={onUpdateHistogramBundleSelection}
          onRemoveComparedHistogram={onRemoveComparedHistogram}
          onAddComparedHistogram={onAddComparedHistogram}
        />
      );
    case "text":
      if (!state) return null;
      return <TextPanel title={descriptor.label} state={state} />;
    case "svg":
      if (!state) return null;
      return <SvgPanel title={descriptor.label} state={state} />;
    default:
      return null;
  }
};

const PanelCollection = ({ title = null, panelSpecs, panelStates, panelValues = {}, onPanelValueChange = null }) => {
  const [histogramBundlesByPanel, setHistogramBundlesByPanel] = useState({});
  const [histogramBundleUploadErrors, setHistogramBundleUploadErrors] = useState({});
  const renderablePanels = useMemo(
    () => buildRenderablePanels(panelSpecs, panelStates, panelValues),
    [panelSpecs, panelStates, panelValues],
  );
  const sharedHistoryPanelIds = useMemo(
    () =>
      asArray(panelSpecs)
        .filter((spec) => isSharedHistoryTimeseriesPanelSpec(spec))
        .map((spec) => spec?.panel_id)
        .filter((id) => typeof id === "string"),
    [panelSpecs],
  );
  const sharedHistoryPanelIdSet = useMemo(() => new Set(sharedHistoryPanelIds), [sharedHistoryPanelIds]);
  const sharedPdfImagePanelIds = useMemo(
    () =>
      asArray(panelSpecs)
        .filter((spec) => isPdfAdaptationImagePanelSpec(spec))
        .map((spec) => spec?.panel_id)
        .filter((id) => typeof id === "string"),
    [panelSpecs],
  );
  const sharedPdfImagePanelIdSet = useMemo(() => new Set(sharedPdfImagePanelIds), [sharedPdfImagePanelIds]);
  const handleUploadHistogramBundle = useCallback(async (panelId, event) => {
    const file = event?.target?.files?.[0];
    if (!panelId || !file) return;
    try {
      const text = await file.text();
      const parsed = JSON.parse(text);
      const bundle = parseUploadedHistogramBundle(parsed);
      setHistogramBundlesByPanel((current) => {
        const existing = asArray(current?.[panelId]);
        return {
          ...current,
          [panelId]: [
            ...existing,
            {
              id: `overlay-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`,
              label: file.name || `bundle-${existing.length + 1}`,
              histograms: bundle.histograms,
              selectionsByHistogram: {},
            },
          ],
        };
      });
      setHistogramBundleUploadErrors((current) => ({ ...current, [panelId]: null }));
      if (event?.target) event.target.value = "";
    } catch (error) {
      const message = error instanceof Error ? error.message : "Invalid histogram bundle JSON.";
      setHistogramBundleUploadErrors((current) => ({ ...current, [panelId]: message }));
    } finally {
      if (event?.target) event.target.value = "";
    }
  }, []);
  const handleRemoveHistogramBundle = useCallback((panelId, bundleId) => {
    if (!panelId || !bundleId) return;
    setHistogramBundlesByPanel((current) => ({
      ...current,
      [panelId]: asArray(current?.[panelId]).filter((bundle) => bundle?.id !== bundleId),
    }));
  }, []);
  const handleUpdateHistogramBundleSelection = useCallback(
    (
      panelId,
      bundleId,
      histogramName,
      selectedHistograms,
      discreteAlignmentByHistogram = {},
      sortModeByHistogram = {},
    ) => {
      if (!panelId || !bundleId) return;
      const key = histogramSelectionKey(histogramName);
      setHistogramBundlesByPanel((current) => ({
        ...current,
        [panelId]: asArray(current?.[panelId]).map((bundle) => {
          if (bundle?.id !== bundleId) return bundle;
          return {
            ...bundle,
            selectionsByHistogram: {
              ...(isObject(bundle?.selectionsByHistogram) ? bundle.selectionsByHistogram : {}),
              [key]: {
                selectedHistograms: asArray(selectedHistograms)
                  .filter((value) => typeof value === "string")
                  .filter((value, index, values) => values.indexOf(value) === index),
                discreteAlignmentByHistogram: Object.fromEntries(
                  Object.entries(isObject(discreteAlignmentByHistogram) ? discreteAlignmentByHistogram : {})
                    .filter(([name, value]) => typeof name === "string" && typeof value === "string")
                    .map(([name, value]) => [name, value === "by_index" ? "by_index" : "by_key"]),
                ),
                sortModeByHistogram: Object.fromEntries(
                  Object.entries(isObject(sortModeByHistogram) ? sortModeByHistogram : {})
                    .filter(([name]) => typeof name === "string")
                    .map(([name, value]) => [name, normalizeHistogramSortMode(value)]),
                ),
              },
            },
          };
        }),
      }));
    },
    [],
  );
  const handleRemoveComparedHistogram = useCallback((panelId, bundleId, histogramName, comparedIndex) => {
    if (!panelId || !bundleId || !Number.isInteger(comparedIndex)) return;
    setHistogramBundlesByPanel((current) => ({
      ...current,
      [panelId]: asArray(current?.[panelId]).map((bundle) => {
        if (bundle?.id !== bundleId) return bundle;
        const selection = normalizeHistogramSelectionState(bundle, histogramName);
        const nextSelectedHistograms = selection.selectedHistograms.filter((_, index) => index !== comparedIndex);
        return {
          ...bundle,
          selectionsByHistogram: {
            ...(isObject(bundle?.selectionsByHistogram) ? bundle.selectionsByHistogram : {}),
            [selection.key]: {
              selectedHistograms: nextSelectedHistograms,
              discreteAlignmentByHistogram: Object.fromEntries(
                Object.entries(selection.discreteAlignmentByHistogram || {}).filter(([name]) =>
                  nextSelectedHistograms.includes(name),
                ),
              ),
              sortModeByHistogram: Object.fromEntries(
                Object.entries(selection.sortModeByHistogram || {}).filter(([name]) =>
                  nextSelectedHistograms.includes(name),
                ),
              ),
            },
          },
        };
      }),
    }));
  }, []);
  const handleAddComparedHistogram = useCallback((panelId, bundleId, histogramName, currentHistogramIsDiscrete) => {
    if (!panelId || !bundleId) return;
    setHistogramBundlesByPanel((current) => ({
      ...current,
      [panelId]: asArray(current?.[panelId]).map((bundle) => {
        if (bundle?.id !== bundleId) return bundle;
        const selection = normalizeHistogramSelectionState(bundle, histogramName);
        const histogramNames = Object.keys(bundle?.histograms || {}).filter(
          (name) => histogramIsDiscrete(bundle?.histograms?.[name]) === Boolean(currentHistogramIsDiscrete),
        );
        const used = new Set(asArray(selection.selectedHistograms).filter((value) => typeof value === "string"));
        const preferred =
          typeof histogramName === "string" && histogramNames.includes(histogramName) ? histogramName : null;
        const candidate =
          preferred && !used.has(preferred) ? preferred : histogramNames.find((name) => !used.has(name)) || null;
        if (typeof candidate !== "string") return bundle;
        return {
          ...bundle,
          selectionsByHistogram: {
            ...(isObject(bundle?.selectionsByHistogram) ? bundle.selectionsByHistogram : {}),
            [selection.key]: {
              selectedHistograms: [...selection.selectedHistograms, candidate].filter(
                (value, index, values) => values.indexOf(value) === index,
              ),
              discreteAlignmentByHistogram: {
                ...(selection.discreteAlignmentByHistogram || {}),
                [candidate]: "by_key",
              },
              sortModeByHistogram: {
                ...(selection.sortModeByHistogram || {}),
                [candidate]: HISTOGRAM_SORT_CANONICAL,
              },
            },
          },
        };
      }),
    }));
  }, []);
  const handlePanelValueChange = useCallback(
    (panelId, nextValue, shouldTriggerPoll = true) => {
      if (typeof onPanelValueChange !== "function") return;
      if (sharedPdfImagePanelIdSet.has(panelId)) {
        const sharedImageView = extractSharedImageZoom(nextValue);
        if (!sharedImageView) {
          onPanelValueChange(panelId, nextValue, shouldTriggerPoll);
          return;
        }
        const targetIds = sharedPdfImagePanelIds;
        if (targetIds.length <= 1) {
          onPanelValueChange(panelId, nextValue, shouldTriggerPoll);
          return;
        }
        targetIds.forEach((targetId, index) => {
          const sourceValue = targetId === panelId ? nextValue : panelValues?.[targetId];
          const mergedValue = mergeSharedImageZoom(sourceValue, sharedImageView);
          const trigger = shouldTriggerPoll && index === targetIds.length - 1;
          onPanelValueChange(targetId, mergedValue, trigger);
        });
        return;
      }
      if (!sharedHistoryPanelIdSet.has(panelId)) {
        onPanelValueChange(panelId, nextValue, shouldTriggerPoll);
        return;
      }
      const sharedView = extractSharedHistoryView(nextValue);
      if (!sharedView) {
        onPanelValueChange(panelId, nextValue, shouldTriggerPoll);
        return;
      }
      const targetIds = sharedHistoryPanelIds;
      if (targetIds.length <= 1) {
        onPanelValueChange(panelId, nextValue, shouldTriggerPoll);
        return;
      }
      targetIds.forEach((targetId, index) => {
        const sourceValue = targetId === panelId ? nextValue : panelValues?.[targetId];
        const mergedValue = mergeSharedHistoryView(sourceValue, sharedView);
        const trigger = shouldTriggerPoll && index === targetIds.length - 1;
        onPanelValueChange(targetId, mergedValue, trigger);
      });
    },
    [onPanelValueChange, panelValues, sharedHistoryPanelIdSet, sharedHistoryPanelIds, sharedPdfImagePanelIdSet, sharedPdfImagePanelIds],
  );

  return (
    <Box sx={{ mb: 3 }}>
      {title ? (
        <Typography variant="h6" sx={{ mb: 2 }}>
          {title}
        </Typography>
      ) : null}
      {renderablePanels.length === 0 ? <Alert severity="info">No panel data available yet.</Alert> : null}
      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: {
            xs: "minmax(0, 1fr)",
            md: "repeat(12, minmax(0, 1fr))",
          },
          gap: 2,
          alignItems: "start",
        }}
      >
        {renderablePanels.map(({ descriptor, state, value }) => (
          <Box
            key={descriptor.panel_id}
            sx={{
              minWidth: 0,
              gridColumn: panelColumnSpan(descriptor),
            }}
          >
            <PanelRenderer
              descriptor={descriptor}
              state={state}
              value={value}
              onValueChange={handlePanelValueChange}
              histogramBundlesByPanel={histogramBundlesByPanel}
              histogramBundleUploadErrors={histogramBundleUploadErrors}
              onUploadHistogramBundle={handleUploadHistogramBundle}
              onRemoveHistogramBundle={handleRemoveHistogramBundle}
              onUpdateHistogramBundleSelection={handleUpdateHistogramBundleSelection}
              onRemoveComparedHistogram={handleRemoveComparedHistogram}
              onAddComparedHistogram={handleAddComparedHistogram}
            />
          </Box>
        ))}
      </Box>
    </Box>
  );
};

export default PanelCollection;
