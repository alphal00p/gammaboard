import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Alert,
  Box,
  Card,
  CardContent,
  Button,
  FormControl,
  FormControlLabel,
  Switch,
  LinearProgress,
  MenuItem,
  Stack,
  Table as MuiTable,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  Select,
  Typography,
} from "@mui/material";
import ReactECharts from "echarts-for-react";
import prettyMilliseconds from "pretty-ms";
import "../../lib/echarts";
import LatexFormula from "../LatexFormula";
import {
  formatCentralValueWithError,
  formatCompactNumber,
  formatDateTime,
  formatEstimateDisplay,
  formatF64Full,
  formatScientific,
} from "../../utils/formatters";
import { asArray } from "../../utils/collections";

const PANEL_ORDER_RANK = new Map([
  ["sample_progress", 0],
  ["estimate_summary", 1],
  ["real_estimate_history", 3],
  ["imag_estimate_history", 4],
  ["abs_signal_to_noise_history", 5],
  ["gammaloop_histogram_bundle", 20],
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

const buildRenderablePanels = (panelSpecs, panelStates, panelValues) => {
  const stateMap = new Map(asArray(panelStates).map((panel) => [panel.panel_id, panel]));
  const renderablePanels = asArray(panelSpecs).map((spec) => ({
    descriptor: spec,
    state: stateMap.get(spec.panel_id) || null,
    value: panelValues?.[spec.panel_id],
  }));
  const bundlePanel = renderablePanels.find(({ descriptor }) => descriptor?.panel_id === "gammaloop_histogram_bundle");
  const payload = bundlePanel?.state?.payload;
  const histograms = payload?.histograms;
  if (bundlePanel && histograms && typeof histograms === "object" && !Array.isArray(histograms)) {
    const sourcePanelId = bundlePanel?.descriptor?.panel_id || "gammaloop_histogram_bundle";
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
      renderablePanels.push({
        descriptor: {
          panel_id: "gammaloop_selected_histogram",
          label: "Selected Histogram",
          kind: "histogram",
          history: "none",
          width: "full",
        },
        state: {
          panel_id: "gammaloop_selected_histogram",
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
          bins: normalizeGammaLoopHistogramBins(selectedHistogram),
        },
        value: bundlePanel.value ?? null,
      });
    }
  }
  return sortRenderablePanels(renderablePanels);
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

const normalizeGammaLoopHistogramBins = (histogram) => {
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
    };
  });
};

const fitDomain = (values) => {
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

const fitXDomain = (values) => {
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

const fitHistogramXDomain = (bins) => {
  const edges = bins.flatMap((bin) => [bin.start, bin.stop]).filter((value) => Number.isFinite(value));
  return fitXDomain(edges);
};

const SIGNED_LOG_EPSILON = Number.EPSILON;
const HISTOGRAM_POSITIVE_COLOR = "#0a9396";
const HISTOGRAM_NEGATIVE_COLOR = "#ca6702";
const HISTOGRAM_ZERO_COLOR = "#6b7280";

const signedLog10 = (value) => {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return Number.NaN;
  return Math.log10(Math.max(Math.abs(numeric), SIGNED_LOG_EPSILON));
};

const formatSignedLogAxisValue = (value) => {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return "n/a";
  return `10^${formatCompactNumber(numeric, 2)}`;
};

const histogramSignColorFromRaw = (rawValue) => {
  const numeric = Number(rawValue);
  if (!Number.isFinite(numeric)) return HISTOGRAM_ZERO_COLOR;
  if (numeric < 0) return HISTOGRAM_NEGATIVE_COLOR;
  if (numeric > 0) return HISTOGRAM_POSITIVE_COLOR;
  return HISTOGRAM_ZERO_COLOR;
};

const FULL_ZOOM = Object.freeze({ start: 0, end: 100 });
const isSharedHistoryTimeseriesPanelSpec = (spec) => {
  const kind = String(spec?.kind || "");
  const history = String(spec?.history || "");
  return (kind === "scalar_timeseries" || kind === "multi_timeseries") && history !== "none";
};
const HISTORY_X_AXIS_MODE_WALL_TIME = "wall_time";
const HISTORY_X_AXIS_MODE_SAMPLER_UPTIME = "sampler_uptime";
const HISTORY_X_AXIS_MODE_COMPLETED_SAMPLES = "completed_samples";
const HISTORY_X_AXIS_MODE_SET = new Set([
  HISTORY_X_AXIS_MODE_WALL_TIME,
  HISTORY_X_AXIS_MODE_SAMPLER_UPTIME,
  HISTORY_X_AXIS_MODE_COMPLETED_SAMPLES,
]);
const inferXAxisLabel = (panelId) => (String(panelId || "").includes("_history") ? "Nr samples" : null);
const inferNumericXAxisLabel = (panelId, mode = HISTORY_X_AXIS_MODE_WALL_TIME) => {
  if (mode === HISTORY_X_AXIS_MODE_SAMPLER_UPTIME) return "Sampler Uptime";
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
const isObject = (value) => value && typeof value === "object" && !Array.isArray(value);
const formatEtaSeconds = (seconds) => {
  const numericSeconds = Number(seconds);
  if (!Number.isFinite(numericSeconds) || numericSeconds < 0) return null;
  const etaMs = Math.max(0, Math.round(numericSeconds * 1000));
  return prettyMilliseconds(etaMs, {
    colonNotation: true,
    secondsDecimalDigits: 0,
    keepDecimalsOnWholeSeconds: false,
    verbose: false,
  });
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

const lineColors = ["#005f73", "#bb3e03", "#0a9396", "#ae2012", "#ca6702"];
const histogramOverlayColors = ["#9b2226", "#3a86ff", "#ff006e", "#6a994e", "#ff7f11", "#8338ec"];

const isIsoDateTime = (value) => typeof value === "string" && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/.test(value);
const isEstimateValue = (value) =>
  isObject(value) &&
  value.kind === "estimate" &&
  Number.isFinite(Number(value.value)) &&
  Number.isFinite(Number(value.error));

const renderStructuredValue = (value) => {
  if (value == null) return "none";
  if (typeof value === "number") return formatCompactNumber(value);
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "string") {
    if (!value.trim()) return "none";
    if (isIsoDateTime(value)) return formatDateTime(value, value);
    return value;
  }
  return JSON.stringify(value);
};

const formatDebugJson = (value) => {
  try {
    return JSON.stringify(value, null, 2);
  } catch (_error) {
    return String(value);
  }
};

const scalarHeatmapColors = ["rgb(0,0,255)", "rgb(128,200,128)", "rgb(255,0,0)"];

const panelColumnSpan = (descriptor) => {
  if (descriptor?.panel_id === "estimate_summary") {
    return { xs: "1 / -1", md: "1 / -1" };
  }
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

const normalizeZoomRange = (candidate) => {
  const start = Number(candidate?.start);
  const end = Number(candidate?.end);
  if (!Number.isFinite(start) || !Number.isFinite(end)) return null;
  const normalizedStart = Math.max(0, Math.min(100, start));
  const normalizedEnd = Math.max(0, Math.min(100, end));
  if (normalizedEnd < normalizedStart) return { start: normalizedEnd, end: normalizedStart };
  return { start: normalizedStart, end: normalizedEnd };
};

const readZoomFromPanelValue = (value, fallback = FULL_ZOOM) =>
  normalizeZoomRange(isObject(value) ? value.zoom : null) || fallback;
const readYZoomFromPanelValue = (value, fallback = FULL_ZOOM) =>
  normalizeZoomRange(isObject(value) ? value.yZoom : null) || fallback;
const readTailPinnedFromPanelValue = (value, fallback = true) =>
  typeof value?.tailPinned === "boolean" ? value.tailPinned : fallback;
const readHistoryXAxisModeFromPanelValue = (value, fallback = HISTORY_X_AXIS_MODE_WALL_TIME) => {
  const mode = isObject(value) ? value.xAxisMode : null;
  return HISTORY_X_AXIS_MODE_SET.has(mode) ? mode : fallback;
};
const writeZoomPanelValue = (current, zoom, tailPinned = null, yZoom = undefined) => {
  const next = isObject(current) ? { ...current } : {};
  next.zoom = normalizeZoomRange(zoom) || FULL_ZOOM;
  if (yZoom !== undefined) next.yZoom = normalizeZoomRange(yZoom) || FULL_ZOOM;
  if (tailPinned != null) next.tailPinned = Boolean(tailPinned);
  return next;
};

const readHistogramBundleSelectedValue = (value) => {
  if (typeof value === "string") return value;
  if (isObject(value) && typeof value.selected_histogram === "string") return value.selected_histogram;
  return null;
};

const readHistogramBundleView = (value) => {
  if (!isObject(value)) return {};
  return isObject(value.histogram_view) ? value.histogram_view : {};
};

const readHistogramZoomFromPanelValue = (value, fallback = FULL_ZOOM) => {
  const view = readHistogramBundleView(value);
  return normalizeZoomRange(view.zoom) || readZoomFromPanelValue(value, fallback);
};
const readHistogramYZoomFromPanelValue = (value, fallback = FULL_ZOOM) => {
  const view = readHistogramBundleView(value);
  return normalizeZoomRange(view.y_zoom) || readYZoomFromPanelValue(value, fallback);
};

const readHistogramScaleFromPanelValue = (value, axis) => {
  const view = readHistogramBundleView(value);
  const scale = view?.[`${axis}_scale`];
  return scale === "log" ? "log" : "linear";
};

const writeHistogramBundlePanelValue = (
  current,
  {
    selectedHistogram = undefined,
    zoom = undefined,
    yZoom = undefined,
    xScale = undefined,
    yScale = undefined,
    showRelativeError = undefined,
    sortModeByHistogram = undefined,
  } = {},
) => {
  const next = isObject(current) ? { ...current } : {};
  const nextView = isObject(next.histogram_view) ? { ...next.histogram_view } : {};
  if (selectedHistogram !== undefined) {
    next.selected_histogram = selectedHistogram;
  }
  if (zoom !== undefined) {
    nextView.zoom = normalizeZoomRange(zoom) || FULL_ZOOM;
  }
  if (yZoom !== undefined) {
    nextView.y_zoom = normalizeZoomRange(yZoom) || FULL_ZOOM;
  }
  if (xScale !== undefined) {
    nextView.x_scale = xScale === "log" ? "log" : "linear";
  }
  if (yScale !== undefined) {
    nextView.y_scale = yScale === "log" ? "log" : "linear";
  }
  if (showRelativeError !== undefined) {
    nextView.show_relative_error = Boolean(showRelativeError);
  }
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
  if (Object.keys(nextView).length > 0) {
    next.histogram_view = nextView;
  }
  return next;
};

const extractSharedHistoryView = (value) => {
  if (!isObject(value)) return null;
  const zoom = normalizeZoomRange(value.zoom);
  const hasTailPinned = typeof value.tailPinned === "boolean";
  const xAxisMode = HISTORY_X_AXIS_MODE_SET.has(value.xAxisMode) ? value.xAxisMode : null;
  if (!zoom && !hasTailPinned && !xAxisMode) return null;
  const shared = {};
  if (zoom) shared.zoom = zoom;
  if (hasTailPinned) shared.tailPinned = value.tailPinned;
  if (xAxisMode) shared.xAxisMode = xAxisMode;
  return shared;
};

const mergeSharedHistoryView = (current, sharedView) => {
  const next = isObject(current) ? { ...current } : {};
  if (sharedView.zoom) next.zoom = sharedView.zoom;
  if ("tailPinned" in sharedView) next.tailPinned = sharedView.tailPinned;
  if ("xAxisMode" in sharedView && HISTORY_X_AXIS_MODE_SET.has(sharedView.xAxisMode)) {
    next.xAxisMode = sharedView.xAxisMode;
  }
  return next;
};

const isPdfAdaptationImagePanelSpec = (spec) =>
  spec?.kind === "image2d" &&
  typeof spec?.panel_id === "string" &&
  spec.panel_id.startsWith("pdf_adaptation_");

const extractSharedImageZoom = (value) => {
  if (!isObject(value)) return null;
  const zoom = normalizeZoomRange(value.zoom);
  return zoom ? { zoom } : null;
};

const mergeSharedImageZoom = (current, sharedView) => {
  const next = isObject(current) ? { ...current } : {};
  if (sharedView.zoom) next.zoom = sharedView.zoom;
  return next;
};

const zoomRangeChanged = (left, right) =>
  Math.abs((left?.start ?? 0) - (right?.start ?? 0)) > 0.01 ||
  Math.abs((left?.end ?? 100) - (right?.end ?? 100)) > 0.01;

const readDataZoomRanges = (event) => {
  const payloads = Array.isArray(event?.batch) && event.batch.length > 0 ? event.batch : [event];
  const xPayload = payloads.find((payload) => {
    if (!payload || typeof payload !== "object") return false;
    const dataZoomId = typeof payload.dataZoomId === "string" ? payload.dataZoomId : "";
    if (dataZoomId.startsWith("x-")) return true;
    if (Array.isArray(payload.xAxisIndex)) return payload.xAxisIndex.includes(0);
    if (Number.isFinite(Number(payload.xAxisIndex))) return Number(payload.xAxisIndex) === 0;
    return false;
  });
  const yPayload = payloads.find((payload) => {
    if (!payload || typeof payload !== "object") return false;
    const dataZoomId = typeof payload.dataZoomId === "string" ? payload.dataZoomId : "";
    if (dataZoomId.startsWith("y-")) return true;
    if (Array.isArray(payload.yAxisIndex)) return payload.yAxisIndex.includes(0);
    if (Number.isFinite(Number(payload.yAxisIndex))) return Number(payload.yAxisIndex) === 0;
    return false;
  });
  return {
    x: normalizeZoomRange(xPayload),
    y: normalizeZoomRange(yPayload),
  };
};

const visibleXRangeFromZoom = (xDomain, zoomRange) => {
  const xMin = Number(xDomain?.[0]);
  const xMax = Number(xDomain?.[1]);
  const normalizedZoom = normalizeZoomRange(zoomRange);
  if (!Number.isFinite(xMin) || !Number.isFinite(xMax) || !normalizedZoom) return null;
  const span = xMax - xMin;
  if (!Number.isFinite(span) || span <= 0) return null;
  return {
    min: xMin + (span * normalizedZoom.start) / 100,
    max: xMin + (span * normalizedZoom.end) / 100,
  };
};

const visibleXRangeFromZoomWithScale = (xDomain, zoomRange, scale = "linear") => {
  const normalizedZoom = normalizeZoomRange(zoomRange);
  const xMin = Number(xDomain?.[0]);
  const xMax = Number(xDomain?.[1]);
  if (!normalizedZoom || !Number.isFinite(xMin) || !Number.isFinite(xMax)) return null;
  if (scale === "log") {
    const minPositive = Math.max(xMin, Number.EPSILON);
    const maxPositive = Math.max(xMax, minPositive * (1 + Number.EPSILON));
    const logMin = Math.log(minPositive);
    const logMax = Math.log(maxPositive);
    const logSpan = logMax - logMin;
    if (!Number.isFinite(logSpan) || logSpan <= 0) return null;
    const visibleLogMin = logMin + (logSpan * normalizedZoom.start) / 100;
    const visibleLogMax = logMin + (logSpan * normalizedZoom.end) / 100;
    return {
      min: Math.exp(visibleLogMin),
      max: Math.exp(visibleLogMax),
    };
  }
  return visibleXRangeFromZoom(xDomain, normalizedZoom);
};

const buildDataZoom = (
  zoomRange = FULL_ZOOM,
  includeSlider = true,
  includeYInside = true,
  yZoomRange = FULL_ZOOM,
  includeYSlider = true,
) => {
  const normalizedZoom = normalizeZoomRange(zoomRange) || FULL_ZOOM;
  const normalizedYZoom = normalizeZoomRange(yZoomRange) || FULL_ZOOM;
  const zoom = [
    {
      id: "x-inside",
      type: "inside",
      xAxisIndex: [0],
      filterMode: "none",
      throttle: 50,
      start: normalizedZoom.start,
      end: normalizedZoom.end,
    },
  ];
  if (includeSlider) {
    zoom.push({
      id: "x-slider",
      type: "slider",
      xAxisIndex: [0],
      filterMode: "none",
      height: 18,
      bottom: 4,
      start: normalizedZoom.start,
      end: normalizedZoom.end,
    });
  }
  if (includeYInside) {
    zoom.push({
      id: "y-inside",
      type: "inside",
      yAxisIndex: [0],
      filterMode: "none",
      throttle: 50,
      start: normalizedYZoom.start,
      end: normalizedYZoom.end,
    });
  }
  if (includeYSlider) {
    zoom.push({
      id: "y-slider",
      type: "slider",
      yAxisIndex: [0],
      filterMode: "none",
      width: 14,
      right: 2,
      top: 16,
      bottom: 32,
      start: normalizedYZoom.start,
      end: normalizedYZoom.end,
    });
  }
  return zoom;
};

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
  const points = asArray(state?.points)
    .slice()
    .map((point) => remapTimeseriesPointXAxis(point, historyXAxisMode))
    .sort((a, b) => a.x - b.x);
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
          <ReactECharts
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
    points: asArray(item?.points).map((point) => remapTimeseriesPointXAxis(point, historyXAxisMode)),
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
          <ReactECharts
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

const buildHistogramData = (bins) =>
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

const buildHistogramStepData = (bins) => {
  const orderedBins = asArray(bins)
    .slice()
    .sort((left, right) => left.start - right.start);
  const points = [];
  for (const [index, bin] of orderedBins.entries()) {
    points.push({
      x: bin.start,
      y: bin.value,
      error: bin.error,
      rangeLabel: `${formatScientific(bin.start, 4)} → ${formatScientific(bin.stop, 4)}`,
    });
    points.push({
      x: bin.stop,
      y: bin.value,
      error: bin.error,
      rangeLabel: `${formatScientific(bin.start, 4)} → ${formatScientific(bin.stop, 4)}`,
    });
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

const buildHistogramRenderData = (bins, scale) => {
  const stepData = buildHistogramStepData(bins);
  if (scale !== "log") return stepData;
  return stepData.map((point) => ({
    ...point,
    raw_y: Number(point.y),
    y: signedLog10(point.y),
    error: Number.isFinite(point.error) ? point.error : 0,
  }));
};

const buildRelativeErrorStepData = (bins) =>
  buildHistogramStepData(bins)
    .map((point) => {
      const value = Number(point?.y);
      const error = Number(point?.error);
      if (!Number.isFinite(value) || !Number.isFinite(error) || value === 0) {
        return {
          ...point,
          relative_error: null,
          positive_relative_error: null,
          negative_relative_error: null,
        };
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

const buildDiscreteRelativeErrorData = (bins) =>
  asArray(bins).map((bin, index) => {
    const value = Number(bin?.value);
    const error = Number(bin?.error);
    if (!Number.isFinite(value) || !Number.isFinite(error) || value === 0) {
      return {
        index,
        relative_error: null,
        positive_relative_error: null,
        negative_relative_error: null,
      };
    }
    const relativeError = Math.abs(error / value);
    return {
      index,
      relative_error: relativeError,
      positive_relative_error: relativeError,
      negative_relative_error: -relativeError,
    };
  });

const discreteHistogramBinKey = (bin, index) => {
  if (bin?.label != null) return `label:${String(bin.label)}`;
  if (bin?.bin_id != null) return `id:${String(bin.bin_id)}`;
  const start = Number(bin?.start);
  if (Number.isFinite(start)) return `start:${start}`;
  return `index:${index}`;
};

const histogramIsDiscrete = (histogram) =>
  asArray(histogram?.bins).some((bin) => bin && (bin.label != null || bin.bin_id != null));

const normalizeUploadedHistogram = (name, histogram) => {
  if (!isObject(histogram)) {
    throw new Error(`histogram '${name}' must be an object`);
  }
  const bins = asArray(histogram?.bins);
  if (bins.length === 0) {
    throw new Error(`histogram '${name}' must contain at least one bin`);
  }

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
    })),
  };
};

const parseUploadedHistogramBundle = (payload) => {
  if (!isObject(payload)) {
    throw new Error("bundle payload must be a JSON object");
  }
  const histograms = payload?.histograms;
  if (!isObject(histograms) || Array.isArray(histograms)) {
    throw new Error("bundle payload must contain a 'histograms' object");
  }
  const entries = Object.entries(histograms).filter(
    ([name, histogram]) => typeof name === "string" && name.trim().length > 0 && isObject(histogram),
  );
  if (entries.length === 0) {
    throw new Error("bundle payload contains no valid histograms");
  }
  const normalizedHistograms = {};
  entries.forEach(([name, histogram]) => {
    normalizedHistograms[name] = normalizeUploadedHistogram(name, histogram);
  });
  return {
    primaryHistogramName: typeof payload?.primary_histogram_name === "string" ? payload.primary_histogram_name : null,
    histograms: normalizedHistograms,
  };
};

const histogramSelectionKey = (histogramName) =>
  typeof histogramName === "string" && histogramName.trim().length > 0 ? histogramName : "__default__";

const HISTOGRAM_SORT_CANONICAL = "canonical";
const HISTOGRAM_SORT_BY_VALUE = "by_value";
const HISTOGRAM_SORT_BY_ABS_VALUE = "by_abs_value";

const normalizeHistogramSortMode = (value) => {
  if (value === HISTOGRAM_SORT_BY_VALUE) return HISTOGRAM_SORT_BY_VALUE;
  if (value === HISTOGRAM_SORT_BY_ABS_VALUE) return HISTOGRAM_SORT_BY_ABS_VALUE;
  return HISTOGRAM_SORT_CANONICAL;
};

const sortHistogramBinsByMode = (bins, sortMode) => {
  const normalizedMode = normalizeHistogramSortMode(sortMode);
  if (normalizedMode === HISTOGRAM_SORT_CANONICAL) return asArray(bins);
  return asArray(bins)
    .map((bin, index) => ({
      bin,
      index,
      value: Number(bin?.value),
    }))
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

const normalizeHistogramSelectionState = (bundle, histogramName) => {
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

const toExponential8 = (value) => {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? numeric.toExponential(8) : "0.00000000e+00";
};

const downloadTextFile = (filename, contents, mimeType = "text/plain;charset=utf-8") => {
  const blob = new Blob([contents], { type: mimeType });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.rel = "noreferrer";
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  URL.revokeObjectURL(url);
};

const sanitizeFigureFilename = (value, fallback = "figure") => {
  const text = String(value ?? "").trim();
  const normalized = text
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return normalized || fallback;
};

const escapeXml = (value) =>
  String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&apos;");

const downloadJsonFile = (baseName, payload) => {
  downloadTextFile(
    `${sanitizeFigureFilename(baseName)}.json`,
    `${JSON.stringify(payload, null, 2)}\n`,
    "application/json;charset=utf-8",
  );
};

const downloadSvgFromElement = (baseName, element) => {
  const svg = element?.querySelector?.("svg");
  if (!svg) return false;
  const serializer = new XMLSerializer();
  const cloned = svg.cloneNode(true);
  cloned.setAttribute("xmlns", "http://www.w3.org/2000/svg");
  const markup = serializer.serializeToString(cloned);
  downloadTextFile(`${sanitizeFigureFilename(baseName)}.svg`, markup, "image/svg+xml;charset=utf-8");
  return true;
};

const downloadSvgFromEcharts = (baseName, echartsRef) => {
  const instance = echartsRef?.current?.getEchartsInstance?.();
  if (!instance) return false;
  try {
    const dataUrl = instance.getDataURL({
      type: "svg",
      pixelRatio: 2,
      backgroundColor: "#ffffff",
    });
    if (typeof dataUrl !== "string" || !dataUrl.startsWith("data:image/svg+xml")) return false;
    const encoded = dataUrl.slice(dataUrl.indexOf(",") + 1);
    const markup = decodeURIComponent(encoded);
    downloadTextFile(`${sanitizeFigureFilename(baseName)}.svg`, markup, "image/svg+xml;charset=utf-8");
    return true;
  } catch {
    return false;
  }
};

const downloadCanvasAsSvg = (baseName, canvas) => {
  if (!canvas?.toDataURL) return false;
  const width = Number(canvas.width) || 1;
  const height = Number(canvas.height) || 1;
  const pngDataUri = canvas.toDataURL("image/png");
  const svg = [
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">`,
    `<image href="${pngDataUri}" x="0" y="0" width="${width}" height="${height}" />`,
    "</svg>",
  ].join("");
  downloadTextFile(`${sanitizeFigureFilename(baseName)}.svg`, svg, "image/svg+xml;charset=utf-8");
  return true;
};

const downloadCanvasCollectionAsSvg = (baseName, canvases) => {
  const list = Array.from(canvases || []).filter((canvas) => canvas?.toDataURL);
  if (list.length === 0) return false;
  const width = Math.max(...list.map((canvas) => Number(canvas.width) || 1), 1);
  const heights = list.map((canvas) => Number(canvas.height) || 1);
  const totalHeight = heights.reduce((sum, height) => sum + height, 0);
  let yOffset = 0;
  const images = list
    .map((canvas, index) => {
      const height = heights[index];
      const pngDataUri = canvas.toDataURL("image/png");
      const imageTag = `<image href="${pngDataUri}" x="0" y="${yOffset}" width="${width}" height="${height}" />`;
      yOffset += height;
      return imageTag;
    })
    .join("");
  const svg = [
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${totalHeight}" viewBox="0 0 ${width} ${totalHeight}">`,
    images,
    "</svg>",
  ].join("");
  downloadTextFile(`${sanitizeFigureFilename(baseName)}.svg`, svg, "image/svg+xml;charset=utf-8");
  return true;
};

const FigureExportActions = ({
  baseName,
  payload,
  elementRef = null,
  echartsRef = null,
  svgBuilder = null,
  onResetView = null,
}) => {
  const handleDownloadSvg = () => {
    if (downloadSvgFromEcharts(baseName, echartsRef)) return;
    if (downloadSvgFromElement(baseName, elementRef?.current)) return;
    if (downloadCanvasCollectionAsSvg(baseName, elementRef?.current?.querySelectorAll?.("canvas"))) return;
    if (downloadCanvasAsSvg(baseName, elementRef?.current?.querySelector?.("canvas"))) return;
    if (typeof svgBuilder === "function") {
      const markup = svgBuilder();
      if (typeof markup === "string" && markup.trim()) {
        downloadTextFile(`${sanitizeFigureFilename(baseName)}.svg`, markup, "image/svg+xml;charset=utf-8");
      }
    }
  };

  return (
    <Stack direction="row" spacing={1} alignItems="center">
      {typeof onResetView === "function" ? (
        <Button size="small" variant="outlined" onClick={onResetView}>
          Reset
        </Button>
      ) : null}
      <Button size="small" variant="outlined" onClick={() => downloadJsonFile(baseName, payload)}>
        JSON
      </Button>
      <Button size="small" variant="outlined" onClick={handleDownloadSvg}>
        SVG
      </Button>
    </Stack>
  );
};

const buildHistogramHwUBlock = (name, histogram) => {
  const bins = asArray(histogram?.bins);
  const title = histogram?.title ?? name ?? "histogram";
  const xAxisMode = histogram?.log_x_axis ? "LOG" : "LIN";
  const yAxisMode = histogram?.log_y_axis ? "LOG" : "LIN";
  const typeDescription = histogram?.type_description ?? "HwU";
  const xMin = Number.isFinite(Number(histogram?.x_min)) ? Number(histogram.x_min) : Number(bins[0]?.start);
  const xMax = Number.isFinite(Number(histogram?.x_max))
    ? Number(histogram.x_max)
    : Number(bins[bins.length - 1]?.stop);
  return [
    "##& xmin & xmax & central value & dy &",
    "",
    `<histogram> ${bins.length} "${title} |X_AXIS@${xAxisMode} |Y_AXIS@${yAxisMode} |TYPE@${typeDescription}"`,
    ...bins.map((bin) =>
      [
        `  ${toExponential8(Number(bin?.start) ?? xMin)}`,
        `${toExponential8(Number(bin?.stop) ?? xMax)}`,
        `${toExponential8(Number(bin?.value) ?? 0)}`,
        `${toExponential8(Number(bin?.error) ?? 0)}`,
      ].join("   "),
    ),
    "<\\histogram>",
    "",
  ].join("\n");
};

const buildHistogramBundleJson = (payload) => ({
  primary_histogram_name: payload?.primary_histogram_name ?? null,
  histograms: payload?.histograms ?? {},
});

const buildHistogramBundleHwU = (payload) => {
  const histograms = payload?.histograms && typeof payload.histograms === "object" ? payload.histograms : {};
  return Object.entries(histograms)
    .map(([name, histogram]) => buildHistogramHwUBlock(name, histogram))
    .join("\n");
};

const buildHistogramYDomain = (bins, scale, visibleXRange = null) => {
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
  if (scale === "log") {
    const transformed = values.map((entry) => signedLog10(entry)).filter((entry) => Number.isFinite(entry));
    return fitDomain(transformed);
  }
  return fitDomain(values);
};

const buildRelativeErrorYDomain = (points, visibleXRange = null) => {
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

const buildCellCenters = (range, count) => {
  const [min, max] = asArray(range);
  if (!Number.isFinite(min) || !Number.isFinite(max) || count <= 0) {
    return Array.from({ length: count }, (_, index) => index);
  }
  const step = (max - min) / count;
  return Array.from({ length: count }, (_, index) => min + step * (index + 0.5));
};

const buildScalarHeatmapScale = (values, normalizationMode) => {
  const finite = values.filter((value) => Number.isFinite(value));
  if (finite.length === 0) {
    return { zmin: 0, zmax: 1 };
  }
  if (normalizationMode === "symmetric") {
    const maxAbs = Math.max(...finite.map((value) => Math.abs(value)), 1e-12);
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
  const xCenters = useMemo(() => buildCellCenters(xRange, width), [width, xRange]);
  const yCenters = useMemo(() => buildCellCenters(yRange, height), [height, yRange]);
  const totalCells = Math.max(0, width * height);
  const boundedValues = useMemo(() => values.slice(0, totalCells), [totalCells, values]);
  const { zmin, zmax } = useMemo(
    () => buildScalarHeatmapScale(boundedValues, normalizationMode),
    [boundedValues, normalizationMode],
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
        name: "x",
        axisLine: { show: true, lineStyle: { color: "#94a3b8" } },
        axisTick: { show: true },
        axisLabel: {
          color: "#64748b",
          fontSize: 11,
          formatter: (value) => {
            const index = Number(value);
            const x = Number.isFinite(index) ? xCenters[Math.max(0, Math.min(width - 1, index))] : Number.NaN;
            return Number.isFinite(x) ? formatScientific(x, 2) : "";
          },
          interval: Math.max(0, Math.ceil(width / 12) - 1),
        },
      },
      yAxis: {
        type: "category",
        data: Array.from({ length: height }, (_, index) => index),
        name: "y",
        axisLine: { show: true, lineStyle: { color: "#94a3b8" } },
        axisTick: { show: true },
        axisLabel: {
          color: "#64748b",
          fontSize: 11,
          formatter: (value) => {
            const index = Number(value);
            const y = Number.isFinite(index) ? yCenters[Math.max(0, Math.min(height - 1, index))] : Number.NaN;
            return Number.isFinite(y) ? formatScientific(y, 2) : "";
          },
          interval: Math.max(0, Math.ceil(height / 12) - 1),
        },
        inverse: true,
      },
      tooltip: {
        trigger: "item",
        formatter: (params) => {
          if (params?.seriesName === "invalid") return "invalid value";
          const data = Array.isArray(params?.data) ? params.data : [];
          const [col, row, value] = data;
          const x = Number.isFinite(Number(col)) ? xCenters[Math.max(0, Math.min(width - 1, Number(col)))] : Number.NaN;
          const y = Number.isFinite(Number(row))
            ? yCenters[Math.max(0, Math.min(height - 1, Number(row)))]
            : Number.NaN;
          return [
            `x: ${formatScientific(Number(x), 4)}`,
            `y: ${formatScientific(Number(y), 4)}`,
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
      xCenters,
      zoomRange,
      yZoomRange,
      yCenters,
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
        </Box>
        <Box
          ref={figureRef}
          sx={{
            width: "min(100%, 920px)",
            mx: "auto",
            height: `${chartHeight}px`,
          }}
        >
          <ReactECharts
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
  const sourcePanelId = state?.source_panel_id || panelId;
  const isBundleControlled = sourcePanelId === "gammaloop_histogram_bundle";
  const currentHistogramName = typeof state?.name === "string" ? state.name : null;
  const [localYScale, setLocalYScale] = useState("linear");
  const [localXScale, setLocalXScale] = useState("linear");
  const [localSortMode, setLocalSortMode] = useState(HISTOGRAM_SORT_CANONICAL);
  const [localShowRelativeErrors, setLocalShowRelativeErrors] = useState(() => {
    const pid = state?.panel_id || "";
    return String(pid).startsWith("pdf_adaptation_") ? false : true;
  });
  const yScale = isBundleControlled ? readHistogramScaleFromPanelValue(value, "y") : localYScale;
  const xScale = isBundleControlled ? readHistogramScaleFromPanelValue(value, "x") : localXScale;
  const zoomRange = isBundleControlled
    ? readHistogramZoomFromPanelValue(value, FULL_ZOOM)
    : readZoomFromPanelValue(value, FULL_ZOOM);
  const yZoomRange = isBundleControlled
    ? readHistogramYZoomFromPanelValue(value, FULL_ZOOM)
    : readYZoomFromPanelValue(value, FULL_ZOOM);
  const view = isBundleControlled ? readHistogramBundleView(value) : {};
  const showRelativeErrors = isBundleControlled ? view.show_relative_error !== false : localShowRelativeErrors;
  const requestedSortMode = isBundleControlled
    ? normalizeHistogramSortMode(
        isObject(view?.sort_mode_by_histogram) && currentHistogramName
          ? view.sort_mode_by_histogram[currentHistogramName]
          : HISTOGRAM_SORT_CANONICAL,
      )
    : normalizeHistogramSortMode(localSortMode);
  const baseCanonicalBins = useMemo(() => buildHistogramData(state?.bins), [state?.bins]);
  const bins = useMemo(() => {
    const isDiscreteHistogram = baseCanonicalBins.some((bin) => bin && (bin.label != null || bin.bin_id != null));
    return isDiscreteHistogram ? sortHistogramBinsByMode(baseCanonicalBins, requestedSortMode) : baseCanonicalBins;
  }, [baseCanonicalBins, requestedSortMode]);
  const stepData = useMemo(() => buildHistogramRenderData(state?.bins, yScale), [state?.bins, yScale]);
  const relativeErrorData = useMemo(() => buildRelativeErrorStepData(state?.bins), [state?.bins]);
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
  const effectiveXScale = isDiscrete ? "linear" : xScale;
  const discreteBaseKeys = useMemo(() => bins.map((bin, index) => discreteHistogramBinKey(bin, index)), [bins]);
  const discreteBaseSortedCanonicalIndices = useMemo(() => {
    if (!isDiscrete) return [];
    const canonicalIndexByBin = new Map(asArray(baseCanonicalBins).map((bin, index) => [bin, index]));
    return asArray(bins).map((bin) => {
      const index = canonicalIndexByBin.get(bin);
      return Number.isFinite(index) ? index : -1;
    });
  }, [baseCanonicalBins, bins, isDiscrete]);
  const comparedBundleSelections = useMemo(
    () =>
      asArray(uploadedBundles).map((bundle) => ({
        bundle,
        selectionState: normalizeHistogramSelectionState(bundle, currentHistogramName),
      })),
    [currentHistogramName, uploadedBundles],
  );
  const overlaySeries = useMemo(
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
              const values =
                discreteMatchMode === "by_index"
                  ? discreteBaseSortedCanonicalIndices.map((sourceIndex) => {
                      const bin = sourceIndex >= 0 ? overlayCanonicalBins[sourceIndex] : null;
                      if (!bin) return null;
                      const numeric = Number(bin?.value);
                      if (!Number.isFinite(numeric)) return null;
                      return yScale === "log" ? signedLog10(numeric) : numeric;
                    })
                  : (() => {
                      return discreteBaseKeys.map((key) => {
                        const bin = overlayBinByKey.get(key);
                        if (!bin) return null;
                        const numeric = Number(bin?.value);
                        if (!Number.isFinite(numeric)) return null;
                        return yScale === "log" ? signedLog10(numeric) : numeric;
                      });
                    })();
              const relative =
                discreteMatchMode === "by_index"
                  ? discreteBaseSortedCanonicalIndices.map((sourceIndex) => {
                      const bin = sourceIndex >= 0 ? overlayCanonicalBins[sourceIndex] : null;
                      if (!bin) return null;
                      const value = Number(bin?.value);
                      const error = Number(bin?.error);
                      if (!Number.isFinite(value) || !Number.isFinite(error) || value === 0) return null;
                      return Math.abs(error / value);
                    })
                  : (() => {
                      return discreteBaseKeys.map((key) => {
                        const bin = overlayBinByKey.get(key);
                        if (!bin) return null;
                        const value = Number(bin?.value);
                        const error = Number(bin?.error);
                        if (!Number.isFinite(value) || !Number.isFinite(error) || value === 0) return null;
                        return Math.abs(error / value);
                      });
                    })();
              const absoluteError = values.map((value, index) => {
                if (!Number.isFinite(value)) return null;
                const sourceBin =
                  discreteMatchMode === "by_index"
                    ? (() => {
                        const sourceIndex = discreteBaseSortedCanonicalIndices[index];
                        return sourceIndex >= 0 ? overlayCanonicalBins[sourceIndex] : null;
                      })()
                    : (() => {
                        const key = discreteBaseKeys[index];
                        return key ? overlayBinByKey.get(key) || null : null;
                      })();
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
              };
            }
            const valueStep = buildHistogramRenderData(histogram.bins, yScale)
              .map((point) => [Number(point?.x), Number(point?.y)])
              .filter(([x]) => Number.isFinite(x) && (effectiveXScale !== "log" || x > 0));
            const relativeStep = buildRelativeErrorStepData(histogram.bins)
              .map((point) => [Number(point?.x), Number(point?.relative_error)])
              .filter(([x]) => Number.isFinite(x) && (effectiveXScale !== "log" || x > 0));
            const absError = buildHistogramData(histogram.bins)
              .map((bin) => {
                const x = Number(bin?.x);
                const y = Number(bin?.value);
                const err = Number(bin?.error);
                if (!Number.isFinite(x) || !Number.isFinite(y) || !Number.isFinite(err) || err <= 0) return null;
                const yLowRaw = y - Math.abs(err);
                const yHighRaw = y + Math.abs(err);
                const yLow = yScale === "log" ? signedLog10(yLowRaw) : yLowRaw;
                const yHigh = yScale === "log" ? signedLog10(yHighRaw) : yHighRaw;
                if (effectiveXScale === "log" && x <= 0) return null;
                return [x, yLow, yHigh];
              })
              .filter(Boolean);
            return {
              id: `${bundle.id}-${selectedName}`,
              name: seriesLabel,
              color,
              valueStep,
              relativeStep,
              absError,
            };
          });
        })
        .filter(Boolean),
    [
      comparedBundleSelections,
      discreteBaseKeys,
      discreteBaseSortedCanonicalIndices,
      effectiveXScale,
      isDiscrete,
      yScale,
    ],
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
  const yDomain = useMemo(() => buildHistogramYDomain(bins, yScale, visibleXRange), [bins, yScale, visibleXRange]);
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
            if (Array.isArray(overlay.discreteAbsError) && overlay.discreteAbsError.length > 0) {
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
              type: "line",
              name: "value (+)",
              data: positiveSeriesData,
              step: "end",
              showSymbol: false,
              lineStyle: { width: 1.35, color: HISTOGRAM_POSITIVE_COLOR },
              itemStyle: { color: HISTOGRAM_POSITIVE_COLOR },
              connectNulls: false,
            },
            {
              type: "line",
              name: "value (-)",
              data: negativeSeriesData,
              step: "end",
              showSymbol: false,
              lineStyle: { width: 1.35, color: HISTOGRAM_NEGATIVE_COLOR },
              itemStyle: { color: HISTOGRAM_NEGATIVE_COLOR },
              connectNulls: false,
            },
            {
              type: "line",
              name: "value (0)",
              data: zeroSeriesData,
              step: "end",
              showSymbol: false,
              lineStyle: { width: 1.35, color: HISTOGRAM_ZERO_COLOR },
              itemStyle: { color: HISTOGRAM_ZERO_COLOR },
              connectNulls: false,
            },
          ]
        : [
            {
              type: "line",
              name: "value",
              data: valueSeriesData,
              step: "end",
              showSymbol: false,
              lineStyle: { width: 1.35, color: "#005f73" },
              connectNulls: false,
            },
          ];
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
        min: yDomain[0],
        max: yDomain[1],
        axisLabel: yScale === "log" ? { ...baseAxisLabel, formatter: formatSignedLogAxisValue } : baseAxisLabel,
        splitLine: { lineStyle: { color: gridColor } },
      },
      tooltip: {
        trigger: "axis",
        valueFormatter: (axisValue) => {
          const numeric = Number(axisValue);
          if (!Number.isFinite(numeric)) return "n/a";
          return yScale === "log" ? formatSignedLogAxisValue(numeric) : formatScientific(numeric, 6);
        },
      },
      dataZoom: buildDataZoom(zoomRange, false, true, yZoomRange, true),
      series: [
        ...(showRelativeErrors && Array.isArray(binErrorData) && binErrorData.length > 0
          ? [buildErrorBarSeries({ name: "error", data: binErrorData })]
          : []),
        ...valueSeries,
        ...overlaySeries.flatMap((overlay) => [
          ...(Array.isArray(overlay.absError) && overlay.absError.length > 0
            ? [buildErrorBarSeries({ name: `${overlay.name} error`, data: overlay.absError, color: overlay.color })]
            : []),
          {
            type: "line",
            name: overlay.name,
            data: overlay.valueStep,
            step: "end",
            showSymbol: false,
            connectNulls: false,
            lineStyle: { width: 1.4, type: "dashed", color: overlay.color },
            itemStyle: { color: overlay.color },
            emphasis: { focus: "series" },
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
            <ReactECharts
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
                <ReactECharts
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
        </Box>
      </CardContent>
    </Card>
  );
};

const TablePanel = ({
  title,
  state,
  uploadedBundles = [],
  onUploadBundle = null,
  onRemoveBundle = null,
  bundleUploadError = null,
}) => {
  const uploadInputRef = useRef(null);
  const columns = asArray(state?.columns);
  const rows = asArray(state?.rows);
  const isGammaLoopBundle = state?.panel_id === "gammaloop_histogram_bundle";
  if (columns.length === 0 || rows.length === 0) {
    if (isGammaLoopBundle) {
      return (
        <Card variant="outlined">
          <CardContent>
            <Typography variant="subtitle1" sx={{ mb: 1 }}>
              {title}
            </Typography>
            {isGammaLoopBundle ? (
              <Box sx={{ mb: 1.5 }}>
                <input
                  ref={uploadInputRef}
                  type="file"
                  accept="application/json,.json"
                  style={{ display: "none" }}
                  onChange={(event) => onUploadBundle?.(state?.panel_id, event)}
                />
                <Stack direction="row" spacing={1} alignItems="center" sx={{ mb: bundleUploadError ? 1 : 0 }}>
                  <Button size="small" variant="outlined" onClick={() => uploadInputRef.current?.click()}>
                    Upload Bundle
                  </Button>
                  {asArray(uploadedBundles).map((bundle) => (
                    <Button
                      key={bundle.id}
                      size="small"
                      variant="text"
                      color="error"
                      onClick={() => onRemoveBundle?.(state?.panel_id, bundle.id)}
                    >
                      Remove {bundle.label}
                    </Button>
                  ))}
                </Stack>
                {bundleUploadError ? <Alert severity="error">{bundleUploadError}</Alert> : null}
              </Box>
            ) : null}
            <Alert severity="warning">
              GammaLoop histogram bundle is empty or incompatible with the current payload shape. Check backend
              task-output errors for accumulator decode details.
            </Alert>
            <Box sx={{ mt: 1.5 }}>
              <Typography variant="caption" color="text.secondary">
                Debug: raw panel state from API
              </Typography>
              <Box
                component="pre"
                sx={{
                  mt: 0.75,
                  mb: 0,
                  p: 1,
                  maxHeight: 220,
                  overflow: "auto",
                  backgroundColor: "rgba(15, 23, 42, 0.04)",
                  borderRadius: 1,
                  fontSize: "0.75rem",
                  lineHeight: 1.35,
                }}
              >
                {formatDebugJson({
                  panel_id: state?.panel_id,
                  columns: state?.columns,
                  rows: state?.rows,
                  payload: state?.payload,
                  selected_value: state?.selected_value,
                })}
              </Box>
            </Box>
          </CardContent>
        </Card>
      );
    }
    return null;
  }
  const columnKeys = columns.map((column) =>
    String(column || "")
      .trim()
      .toLowerCase(),
  );
  const centralValueIndex = columnKeys.findIndex((column) => column === "central value");
  const errorIndex = columnKeys.findIndex((column) => column === "dy" || column === "error");
  const payload = state?.payload;
  const selectableRows =
    payload?.histograms && typeof payload.histograms === "object" && !Array.isArray(payload.histograms);
  const bundleJson = selectableRows ? buildHistogramBundleJson(payload) : null;
  const handleDownloadJson = () => {
    const filename = `${state?.panel_id ?? "histogram_bundle"}.json`;
    downloadTextFile(filename, `${JSON.stringify(bundleJson, null, 2)}\n`, "application/json;charset=utf-8");
  };
  const handleDownloadHwU = () => {
    const filename = `${state?.panel_id ?? "histogram_bundle"}.HwU`;
    downloadTextFile(filename, buildHistogramBundleHwU(payload));
  };
  const renderTableCell = (row, columnIndex) => {
    if (columnIndex === centralValueIndex && errorIndex >= 0) {
      return formatCentralValueWithError(row?.[columnIndex], row?.[errorIndex], "n/a");
    }
    return renderStructuredValue(row?.[columnIndex]);
  };
  return (
    <Card variant="outlined">
      <CardContent>
        <Box sx={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 2, mb: 2 }}>
          <Typography variant="subtitle1">{title}</Typography>
          {selectableRows ? (
            <Stack direction="row" spacing={1} alignItems="center">
              <Button size="small" variant="outlined" onClick={handleDownloadJson}>
                JSON
              </Button>
              <Button size="small" variant="outlined" onClick={handleDownloadHwU}>
                HwU
              </Button>
            </Stack>
          ) : null}
        </Box>
        {isGammaLoopBundle ? (
          <Box sx={{ mb: 1.5, display: "grid", gap: 1 }}>
            <input
              ref={uploadInputRef}
              type="file"
              accept="application/json,.json"
              style={{ display: "none" }}
              onChange={(event) => onUploadBundle?.(state?.panel_id, event)}
            />
            <Stack direction="row" spacing={1} alignItems="center" sx={{ flexWrap: "wrap" }}>
              <Button size="small" variant="outlined" onClick={() => uploadInputRef.current?.click()}>
                Upload Bundle
              </Button>
              {asArray(uploadedBundles).map((bundle) => (
                <Button
                  key={bundle.id}
                  size="small"
                  variant="text"
                  color="error"
                  onClick={() => onRemoveBundle?.(state?.panel_id, bundle.id)}
                >
                  Remove {bundle.label}
                </Button>
              ))}
            </Stack>
            {bundleUploadError ? <Alert severity="error">{bundleUploadError}</Alert> : null}
          </Box>
        ) : null}
        <TableContainer sx={{ maxHeight: 440, overflowX: "auto" }}>
          <MuiTable size="small" stickyHeader>
            <TableHead>
              <TableRow>
                {columns.map((column) => (
                  <TableCell key={column} sx={{ fontWeight: 600, whiteSpace: "nowrap" }}>
                    {column}
                  </TableCell>
                ))}
              </TableRow>
            </TableHead>
            <TableBody>
              {rows.map((row, rowIndex) => (
                <TableRow
                  key={`row-${rowIndex}`}
                  hover={selectableRows}
                  selected={
                    selectableRows &&
                    String(row?.[0] ?? "") ===
                      String(
                        isGammaLoopBundle
                          ? readHistogramBundleSelectedValue(state?.selected_value)
                          : state?.selected_value,
                      )
                  }
                  sx={{
                    cursor: selectableRows ? "pointer" : "default",
                  }}
                  onClick={
                    selectableRows && typeof row?.[0] === "string"
                      ? () =>
                          state?.onValueChange?.(
                            state?.panel_id,
                            isGammaLoopBundle
                              ? writeHistogramBundlePanelValue(state?.selected_value, {
                                  selectedHistogram: row[0],
                                })
                              : row[0],
                          )
                      : undefined
                  }
                >
                  {columns.map((_, columnIndex) => (
                    <TableCell
                      key={`${rowIndex}-${columnIndex}`}
                      sx={{
                        fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, Liberation Mono, monospace",
                        whiteSpace: "pre-wrap",
                        wordBreak: "break-word",
                        verticalAlign: "top",
                      }}
                    >
                      {renderTableCell(row, columnIndex)}
                    </TableCell>
                  ))}
                </TableRow>
              ))}
            </TableBody>
          </MuiTable>
        </TableContainer>
      </CardContent>
    </Card>
  );
};

const ProgressPanel = ({ title, state }) => {
  const current = Number(state?.current);
  const total = Number(state?.total);
  const eta = formatEtaSeconds(state?.eta_seconds);
  const progress = Number.isFinite(current) && Number.isFinite(total) && total > 0 ? (current / total) * 100 : 0;
  return (
    <Card variant="outlined">
      <CardContent>
        <Typography variant="subtitle1" sx={{ mb: 2 }}>
          {title}
        </Typography>
        <Typography variant="h5" sx={{ fontFamily: "monospace", mb: 1 }}>
          {Number.isFinite(current) ? current.toLocaleString() : "0"}
          {Number.isFinite(total) ? ` / ${total.toLocaleString()}` : ""}
        </Typography>
        <LinearProgress
          variant={Number.isFinite(total) && total > 0 ? "determinate" : "indeterminate"}
          value={progress}
        />
        {eta ? (
          <Typography variant="body2" color="text.secondary" sx={{ mt: 1 }}>
            ETA: {eta}
          </Typography>
        ) : null}
      </CardContent>
    </Card>
  );
};

const KeyValuePanel = ({ title, state }) => (
  <Card variant="outlined">
    <CardContent>
      <Typography variant="subtitle1" sx={{ mb: 2 }}>
        {title}
      </Typography>
      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: {
            xs: "minmax(0, 1fr)",
            lg: "minmax(0, 1fr) minmax(0, 1fr)",
          },
          gap: 1.5,
        }}
      >
        {asArray(state?.entries).map((entry) => (
          <Box
            key={entry.key}
            sx={{
              display: "grid",
              gridTemplateColumns: "minmax(120px, 0.9fr) minmax(0, 1.1fr)",
              gap: 1,
              py: 0.5,
              borderBottom: "1px solid",
              borderColor: "divider",
            }}
          >
            <Typography variant="body2" color="text.secondary">
              {entry.label}
            </Typography>
            {isEstimateValue(entry.value) ? (
              <EstimateValueBlock value={entry.value} />
            ) : (
              <Typography
                variant="body2"
                sx={{
                  fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, Liberation Mono, monospace",
                  wordBreak: "break-word",
                  whiteSpace: "pre-wrap",
                  color:
                    entry.tone === "good"
                      ? "success.main"
                      : entry.tone === "warning"
                        ? "warning.main"
                        : entry.tone === "critical"
                          ? "error.main"
                          : "text.primary",
                }}
              >
                {renderStructuredValue(entry.value)}
              </Typography>
            )}
          </Box>
        ))}
      </Box>
    </CardContent>
  </Card>
);

const EstimateValueBlock = ({ value }) => {
  const central = Number(value?.value);
  const error = Number(value?.error);
  const estimate = formatEstimateDisplay(central, error, "n/a");
  return (
    <Box sx={{ minWidth: 0 }}>
      <Box
        sx={{
          fontSize: "1.05rem",
          fontWeight: 700,
          lineHeight: 1.35,
          whiteSpace: "nowrap",
          overflowX: "auto",
          pb: 0.25,
        }}
      >
        <LatexFormula latex={estimate.latex} display={false} fallbackPrefix="Estimate" />
      </Box>
      <Box component="details" sx={{ mt: 0.5 }}>
        <Box component="summary" sx={{ cursor: "pointer", fontSize: "0.8rem", color: "text.secondary" }}>
          Full precision (f64)
        </Box>
        <Typography
          variant="caption"
          sx={{
            mt: 0.5,
            display: "block",
            fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, Liberation Mono, monospace",
            whiteSpace: "pre-wrap",
          }}
        >
          {`value = ${formatF64Full(central, "n/a")}\nerror = ${formatF64Full(error, "n/a")}`}
        </Typography>
      </Box>
    </Box>
  );
};

const TextPanel = ({ title, state }) => (
  <Card variant="outlined">
    <CardContent>
      <Typography variant="subtitle1" sx={{ mb: 1 }}>
        {title}
      </Typography>
      <Typography variant="body2" color="text.secondary" sx={{ whiteSpace: "pre-wrap", wordBreak: "break-word" }}>
        {renderStructuredValue(state?.text)}
      </Typography>
    </CardContent>
  </Card>
);

const TickBreakdownPanel = ({ title, state }) => {
  const totalMs = Number(state?.total_ms);
  const segments = asArray(state?.segments)
    .map((segment) => ({
      ...segment,
      valueMs: Number(segment?.value_ms),
    }))
    .filter((segment) => Number.isFinite(segment.valueMs) && segment.valueMs > 0);
  const normalizedTotal =
    Number.isFinite(totalMs) && totalMs > 0 ? totalMs : segments.reduce((sum, segment) => sum + segment.valueMs, 0);

  if (segments.length === 0 || !Number.isFinite(normalizedTotal) || normalizedTotal <= 0) return null;
  const buildTickBreakdownSvg = () => {
    const width = 960;
    const barHeight = 36;
    const rowHeight = 22;
    const legendTop = 64;
    const height = legendTop + segments.length * rowHeight + 8;
    let offsetX = 0;
    const bars = segments
      .map((segment) => {
        const percent = (segment.valueMs / normalizedTotal) * 100;
        const segmentWidth = Math.max((width * percent) / 100, 2);
        const rect = `<rect x="${offsetX}" y="8" width="${segmentWidth}" height="${barHeight}" fill="${escapeXml(segment.color || "#0f766e")}" />`;
        offsetX += segmentWidth;
        return rect;
      })
      .join("");
    const legend = segments
      .map((segment, index) => {
        const percent = (segment.valueMs / normalizedTotal) * 100;
        const y = legendTop + index * rowHeight;
        return [
          `<rect x="0" y="${y - 10}" width="10" height="10" fill="${escapeXml(segment.color || "#0f766e")}" />`,
          `<text x="16" y="${y - 1}" font-size="12" font-family="monospace" fill="#334155">${escapeXml(segment.label || segment.key)}</text>`,
          `<text x="${width}" y="${y - 1}" text-anchor="end" font-size="12" font-family="monospace" fill="#475569">${escapeXml(`${formatScientific(segment.valueMs, 4)} ms (${formatScientific(percent, 3)}%)`)}</text>`,
        ].join("");
      })
      .join("");
    return [
      `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">`,
      `<text x="0" y="60" font-size="12" font-family="monospace" fill="#64748b">total ${escapeXml(formatScientific(normalizedTotal, 4))} ms</text>`,
      bars,
      legend,
      "</svg>",
    ].join("");
  };

  return (
    <Card variant="outlined">
      <CardContent>
        <Box sx={{ display: "flex", alignItems: "baseline", justifyContent: "space-between", gap: 2, mb: 2 }}>
          <Typography variant="subtitle1">{title}</Typography>
          <Stack direction="row" spacing={1} alignItems="center">
            <FigureExportActions
              baseName={state?.panel_id || title || "tick_breakdown"}
              payload={{ panel_id: state?.panel_id ?? null, kind: "tick_breakdown", state }}
              svgBuilder={buildTickBreakdownSvg}
            />
            <Typography variant="body2" color="text.secondary" sx={{ fontFamily: "monospace" }}>
              total {formatScientific(normalizedTotal, 4)} ms
            </Typography>
          </Stack>
        </Box>
        <Box
          sx={{
            display: "flex",
            width: "100%",
            minHeight: 44,
            borderRadius: 1.5,
            overflow: "hidden",
            border: "1px solid",
            borderColor: "divider",
            backgroundColor: "rgba(15,23,42,0.04)",
          }}
        >
          {segments.map((segment) => {
            const percent = (segment.valueMs / normalizedTotal) * 100;
            const showInlineLabel = percent >= 10;
            return (
              <Box
                key={segment.key}
                title={`${segment.label}: ${formatScientific(segment.valueMs, 4)} ms (${formatScientific(percent, 3)}%)`}
                sx={{
                  width: `${Math.max(percent, 1.5)}%`,
                  minWidth: 0,
                  px: showInlineLabel ? 1 : 0,
                  py: 0.75,
                  display: "flex",
                  alignItems: "center",
                  justifyContent: showInlineLabel ? "space-between" : "center",
                  gap: 1,
                  color: "#fff",
                  backgroundColor: segment.color || "#0f766e",
                }}
              >
                {showInlineLabel ? (
                  <>
                    <Typography variant="caption" sx={{ fontWeight: 600, color: "inherit", lineHeight: 1.15 }}>
                      {segment.label}
                    </Typography>
                    <Typography variant="caption" sx={{ color: "inherit", opacity: 0.95, lineHeight: 1.15 }}>
                      {formatScientific(segment.valueMs, 3)} ms
                    </Typography>
                  </>
                ) : null}
              </Box>
            );
          })}
        </Box>
        <Box
          sx={{
            mt: 1.25,
            display: "grid",
            gridTemplateColumns: { xs: "1fr", md: "1fr 1fr" },
            gap: 0.75,
          }}
        >
          {segments.map((segment) => {
            const percent = (segment.valueMs / normalizedTotal) * 100;
            return (
              <Box key={`${segment.key}-legend`} sx={{ display: "flex", alignItems: "center", gap: 1 }}>
                <Box
                  sx={{
                    width: 10,
                    height: 10,
                    borderRadius: 0.5,
                    backgroundColor: segment.color || "#0f766e",
                    flexShrink: 0,
                  }}
                />
                <Typography variant="caption" color="text.secondary" sx={{ minWidth: 0 }}>
                  {segment.label}
                </Typography>
                <Typography
                  variant="caption"
                  sx={{ ml: "auto", fontFamily: "monospace", color: "text.secondary", whiteSpace: "nowrap" }}
                >
                  {formatScientific(segment.valueMs, 4)} ms ({formatScientific(percent, 3)}%)
                </Typography>
              </Box>
            );
          })}
        </Box>
      </CardContent>
    </Card>
  );
};

const SelectPanel = ({ title, descriptor, value, onValueChange }) => {
  const options = asArray(descriptor?.state?.options);
  return (
    <Card variant="outlined">
      <CardContent>
        <Typography variant="subtitle1" sx={{ mb: 2 }}>
          {title}
        </Typography>
        <FormControl fullWidth size="small">
          <Select value={value ?? ""} onChange={(event) => onValueChange?.(descriptor.panel_id, event.target.value)}>
            {options.map((option) => (
              <MenuItem key={String(option.value)} value={option.value}>
                {option.label}
              </MenuItem>
            ))}
          </Select>
        </FormControl>
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
