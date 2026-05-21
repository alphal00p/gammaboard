import { Suspense, forwardRef, lazy, memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Alert,
  Box,
  Card,
  CardContent,
  Button,
  FormControl,
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
import HistogramPanel from "./HistogramPanel";
import {
  buildHistogramData,
  fitDomain,
  fitXDomain,
  histogramIsDiscrete,
  normalizeGammaLoopHistogramBins,
  readHistogramBundleSelectedValue,
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
import { useHistogramBundles } from "./histogramBundles";

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
    overlay_alignment: "shared_edges",
    overlay_histograms: [
      {
        name: overlayName,
        color: overlayColor,
        suppress_error_bars: true,
        bins: asArray(overlayPanel?.state?.bins),
      },
    ],
    controls: {
      ...(isObject(primaryPanel?.state?.controls) ? primaryPanel.state.controls : {}),
      default_relative_error: false,
    },
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
    const expandsToHistogram =
      payload?.expands_to?.kind === "histogram" && payload?.expands_to?.source === "selected_row";
    return (
      descriptor?.kind === "table" &&
      (expandsToHistogram ||
        // Fallback for older histogram bundle payloads.
        (payload?.histograms &&
          typeof payload.histograms === "object" &&
          !Array.isArray(payload.histograms)))
    );
  });
  const payload = bundlePanel?.state?.payload;
  const histograms = payload?.histograms;
  const expandsToHistogram =
    payload?.expands_to?.kind === "histogram" && payload?.expands_to?.source === "selected_row";
  if (
    bundlePanel &&
    (expandsToHistogram || payload?.expands_to == null) &&
      payload?.histograms &&
      typeof payload.histograms === "object" &&
      !Array.isArray(payload.histograms)
  ) {
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
          metric_descriptors: selectedHistogram.metric_descriptors,
          views: selectedHistogram.views,
          controls: selectedHistogram.controls,
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

const scalarHeatmapColors = ["#1d4ed8", "#16a34a", "#dc2626"];
const HEATMAP_LEGEND_WIDTH = 116;
const HEATMAP_LEGEND_GAP = 12;
const HEATMAP_PROGRESSIVE_THRESHOLD = 256 * 256;

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

const formatAxisValue = (value) => {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? formatScientific(numeric, 3) : "";
};

const clampHeatmapSpread = (candidate, fallback = 1) => {
  const numeric = Number(candidate);
  if (!Number.isFinite(numeric) || numeric <= 0) return fallback;
  return Math.max(0.05, Math.min(20, numeric));
};
const HEATMAP_SPREAD_MIN = 0.05;
const HEATMAP_SPREAD_MAX = 20;
const HEATMAP_SPREAD_LOG_MIN = Math.log10(HEATMAP_SPREAD_MIN);
const HEATMAP_SPREAD_LOG_MAX = Math.log10(HEATMAP_SPREAD_MAX);
const readHeatmapSpreadFromPanelValue = (value, fallback = 1) =>
  clampHeatmapSpread(isObject(value) ? value.spread : null, fallback);
const writeHeatmapSpreadPanelValue = (current, spread) => {
  const next = isObject(current) ? { ...current } : {};
  next.spread = clampHeatmapSpread(spread, 1);
  return next;
};
const extractSharedPdfImageView = (value) => {
  if (!isObject(value)) return null;
  const shared = {};
  const zoom = extractSharedImageZoom(value);
  if (zoom?.zoom) shared.zoom = zoom.zoom;
  if (Number.isFinite(Number(value.spread))) shared.spread = clampHeatmapSpread(value.spread, 1);
  return Object.keys(shared).length > 0 ? shared : null;
};
const mergeSharedPdfImageView = (current, sharedView) => {
  const next = mergeSharedImageZoom(current, sharedView);
  if (Number.isFinite(Number(sharedView?.spread))) next.spread = clampHeatmapSpread(sharedView.spread, 1);
  return next;
};

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
  const zoomRange = useMemo(() => readZoomFromPanelValue(value, FULL_ZOOM), [value]);
  const yZoomRange = useMemo(() => readYZoomFromPanelValue(value, FULL_ZOOM), [value]);
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

const estimateHeatmapChartHeight = (width, height, panelWidth, margins) => {
  if (width <= 0 || height <= 0) return 360;
  const availableWidth =
    panelWidth > 0
      ? panelWidth
      : Math.min(920, Math.max(320, typeof window !== "undefined" ? window.innerWidth - 64 : 920));
  const legendWidth = HEATMAP_LEGEND_WIDTH + HEATMAP_LEGEND_GAP;
  const innerWidth = Math.max(1, availableWidth - legendWidth - margins.left - margins.right);
  const innerHeight = (innerWidth * height) / width;
  return Math.max(220, Math.round(innerHeight + margins.top + margins.bottom));
};

const heatmapMetricLabel = (panelId) => {
  if (typeof panelId !== "string") return "value";
  if (panelId.includes("oversampling")) return "1 - PDF / integrand";
  if (panelId.includes("log_pdf")) return "log10(normalized PDF)";
  if (panelId.includes("log_integrand")) return "log10(normalized integrand)";
  return "value";
};

const heatmapTooltipValueLines = (panelId, value) => {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return ["value: n/a"];
  const label = heatmapMetricLabel(panelId);
  const lines = [`${label}: ${formatScientific(numeric, 6)}`];
  if (typeof panelId === "string" && panelId.includes("oversampling")) {
    const ratio = 1 - numeric;
    if (Number.isFinite(ratio)) lines.push(`PDF / integrand: ${formatScientific(ratio, 6)}`);
  } else if (typeof panelId === "string" && panelId.startsWith("pdf_adaptation_")) {
    const factor = 10 ** numeric;
    if (Number.isFinite(factor)) lines.push(`linear factor: ${formatScientific(factor, 6)}`);
  }
  return lines;
};

const HeatmapScaleLegend = ({ zmin, zmax, normalizationMode, panelId }) => {
  const max = Number(zmax);
  const min = Number(zmin);
  const showMidpoint = normalizationMode === "symmetric" && min < 0 && max > 0;
  const label = heatmapMetricLabel(panelId);
  return (
    <Box sx={{ width: HEATMAP_LEGEND_WIDTH, flexShrink: 0, display: "grid", gap: 0.75 }}>
      <Typography variant="caption" color="text.secondary" sx={{ lineHeight: 1.15 }}>
        {label}
      </Typography>
      <Box sx={{ display: "grid", gridTemplateColumns: "22px 1fr", gap: 1, alignItems: "stretch", height: 220 }}>
        <Box
          sx={{
            width: 22,
            height: "100%",
            borderRadius: 0.75,
            border: "1px solid rgba(100,116,139,0.35)",
            background: `linear-gradient(to top, ${scalarHeatmapColors[0]} 0%, ${scalarHeatmapColors[1]} 50%, ${scalarHeatmapColors[2]} 100%)`,
          }}
        />
        <Box sx={{ position: "relative", minWidth: 0 }}>
          <Typography
            variant="caption"
            sx={{ position: "absolute", top: -3, left: 0, color: "#475569", fontFamily: "monospace" }}
          >
            {formatScientific(max, 3)}
          </Typography>
          {showMidpoint ? (
            <Typography
              variant="caption"
              sx={{
                position: "absolute",
                top: "50%",
                left: 0,
                transform: "translateY(-50%)",
                color: "#166534",
                fontFamily: "monospace",
                fontWeight: 700,
              }}
            >
              0
            </Typography>
          ) : null}
          <Typography
            variant="caption"
            sx={{ position: "absolute", bottom: -3, left: 0, color: "#475569", fontFamily: "monospace" }}
          >
            {formatScientific(min, 3)}
          </Typography>
        </Box>
      </Box>
    </Box>
  );
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
  const suppressChartEventsRef = useRef(false);
  const [panelWidth, setPanelWidth] = useState(0);
  const xParameters = useMemo(() => buildLinspaceParameters(xRange, width), [width, xRange]);
  const yParameters = useMemo(() => buildLinspaceParameters(yRange, height), [height, yRange]);
  const totalCells = Math.max(0, width * height);
  const boundedValues = useMemo(() => values.slice(0, totalCells), [totalCells, values]);
  const isPdfPanel = typeof panelId === "string" && panelId.startsWith("pdf_adaptation_");
  const supportsSpreadControl = normalizationMode === "symmetric" && isPdfPanel && panelId && typeof onValueChange === "function";
  const spread = supportsSpreadControl ? readHeatmapSpreadFromPanelValue(value, 1) : 1;
  const spreadSliderValue = Math.log10(clampHeatmapSpread(spread, 1));
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

  const heatmapMargins = useMemo(() => ({ left: 56, right: 24, top: 16, bottom: 44 }), []);
  const zoomRange = useMemo(() => readZoomFromPanelValue(value, FULL_ZOOM), [value]);
  const yZoomRange = useMemo(() => readYZoomFromPanelValue(value, FULL_ZOOM), [value]);
  const chartHeight = useMemo(() => {
    return estimateHeatmapChartHeight(width, height, panelWidth, heatmapMargins);
  }, [heatmapMargins.bottom, heatmapMargins.left, heatmapMargins.right, heatmapMargins.top, height, panelWidth, width]);
  const useProgressiveHeatmap = totalCells > HEATMAP_PROGRESSIVE_THRESHOLD;

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
        confine: true,
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
            ...heatmapTooltipValueLines(panelId, value),
          ].join("<br/>");
        },
      },
      visualMap: {
        show: false,
        min: zmin,
        max: zmax,
        dimension: 2,
        inRange: { color: scalarHeatmapColors },
      },
      dataZoom: buildDataZoom(zoomRange, true, true, yZoomRange, true),
      series: [
        {
          type: "heatmap",
          name: "value",
          data: heatmapData,
          progressive: useProgressiveHeatmap ? 5000 : 0,
          progressiveThreshold: HEATMAP_PROGRESSIVE_THRESHOLD,
          emphasis: { disabled: true },
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
      panelId,
      width,
      xParameters,
      zoomRange,
      yZoomRange,
      yParameters,
      zmax,
      zmin,
      useProgressiveHeatmap,
    ],
  );
  useEffect(() => {
    suppressChartEventsRef.current = true;
    const rafId = requestAnimationFrame(() => {
      suppressChartEventsRef.current = false;
    });
    return () => cancelAnimationFrame(rafId);
  }, [option]);
  const onDataZoom = useMemo(
    () => ({
      datazoom: (event) => {
        if (suppressChartEventsRef.current) return;
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
    const chart = echartsRef.current?.getEchartsInstance?.();
    if (!chart) return undefined;
    suppressChartEventsRef.current = true;
    let settleRafId = null;
    const rafId = requestAnimationFrame(() => {
      chart.resize();
      settleRafId = requestAnimationFrame(() => {
        suppressChartEventsRef.current = false;
      });
    });
    return () => {
      cancelAnimationFrame(rafId);
      if (settleRafId != null) cancelAnimationFrame(settleRafId);
    };
  }, [chartHeight, panelWidth]);
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
                  min={HEATMAP_SPREAD_LOG_MIN}
                  max={HEATMAP_SPREAD_LOG_MAX}
                  step={0.01}
                  value={spreadSliderValue}
                  onChange={(_event, next) => {
                    const numeric = Array.isArray(next) ? next[0] : next;
                    onValueChange(panelId, writeHeatmapSpreadPanelValue(value, 10 ** Number(numeric)), false);
                  }}
                  valueLabelDisplay="auto"
                  valueLabelFormat={(next) => `${(10 ** Number(next)).toFixed(2)}x`}
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
          <Box sx={{ display: "flex", alignItems: "center", gap: 1.5, height: "100%" }}>
            <Box sx={{ flex: 1, minWidth: 0, height: "100%" }}>
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
            <HeatmapScaleLegend zmin={zmin} zmax={zmax} normalizationMode={normalizationMode} panelId={panelId} />
          </Box>
        </Box>
      </CardContent>
    </Card>
  );
};


const imagePanelPropsEqual = (left, right) =>
  left.title === right.title &&
  left.value === right.value &&
  left.onValueChange === right.onValueChange &&
  left.state?.panel_id === right.state?.panel_id &&
  left.state?.width === right.state?.width &&
  left.state?.height === right.state?.height &&
  left.state?.color_mode === right.state?.color_mode &&
  left.state?.normalization_mode === right.state?.normalization_mode &&
  left.state?.x_range === right.state?.x_range &&
  left.state?.y_range === right.state?.y_range &&
  left.state?.values === right.state?.values &&
  left.state?.imag_values === right.state?.imag_values &&
  left.state?.invalid_indices === right.state?.invalid_indices;

const Image2dPanel = memo(({ title, state, value = undefined, onValueChange = null }) => {
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
}, imagePanelPropsEqual);
Image2dPanel.displayName = "Image2dPanel";

const PanelRenderer = ({
  descriptor,
  state,
  value,
  onValueChange,
  histogramBundlesByPanel,
  histogramBundleUploadErrors,
  uploadHistogramBundle,
  removeHistogramBundle,
  updateHistogramBundleSelection,
  removeComparedHistogram,
  addComparedHistogram,
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
          onUploadBundle={uploadHistogramBundle}
          onRemoveBundle={removeHistogramBundle}
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
          onUpdateBundleSelection={updateHistogramBundleSelection}
          onRemoveComparedHistogram={removeComparedHistogram}
          onAddComparedHistogram={addComparedHistogram}
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
  const {
    histogramBundlesByPanel,
    histogramBundleUploadErrors,
    uploadHistogramBundle,
    removeHistogramBundle,
    updateHistogramBundleSelection,
    removeComparedHistogram,
    addComparedHistogram,
  } = useHistogramBundles();
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
  const handlePanelValueChange = useCallback(
    (panelId, nextValue, shouldTriggerPoll = true) => {
      if (typeof onPanelValueChange !== "function") return;
      if (sharedPdfImagePanelIdSet.has(panelId)) {
        const sharedImageView = extractSharedPdfImageView(nextValue);
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
          const mergedValue = mergeSharedPdfImageView(sourceValue, sharedImageView);
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
              uploadHistogramBundle={uploadHistogramBundle}
              removeHistogramBundle={removeHistogramBundle}
              updateHistogramBundleSelection={updateHistogramBundleSelection}
              removeComparedHistogram={removeComparedHistogram}
              addComparedHistogram={addComparedHistogram}
            />
          </Box>
        ))}
      </Box>
    </Box>
  );
};

export default PanelCollection;
