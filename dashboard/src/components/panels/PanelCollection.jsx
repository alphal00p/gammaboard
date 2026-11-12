import { useWorkspaceActive } from "../common/WorkspaceTabs";
import { PanelHeader } from "../common/PanelChrome";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Alert,
  Box,
  Card,
  CardContent,
  Typography,
} from "@mui/material";
import { formatDecimalSignificant, formatScientific } from "../../utils/formatters";
import { asArray } from "../../utils/collections";
import {
  EmptyPlotPanel,
  KeyValuePanel,
  ProgressPanel,
  SelectPanel,
  SelectInput,
  SvgPanel,
  TextPanel,
  TickBreakdownPanel,
} from "./BasicPanels";
import FigureExportActions, { escapeXml } from "./FigureExportActions";
import TablePanel from "./TablePanel";
import ObservableHistogramPanel from "./ObservableHistogramPanel";
import StrataPanel from "./StrataPanel";
import PdfComparisonPanel from "./PdfComparisonPanel";
import {
  buildHistogramData,
  fitDomain,
  fitXDomain,
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
  isSharedHistoryTimeseriesPanelSpec,
  mergeSharedHistoryView,
  normalizeZoomRange,
  readDataZoomRanges,
  readHistoryXAxisModeFromPanelValue,
  readTailPinnedFromPanelValue,
  readYZoomFromPanelValue,
  readZoomFromPanelValue,
  visibleXRangeFromZoom,
  writeZoomPanelValue,
  zoomRangeChanged,
} from "./panelView";
import { useHistogramBundles } from "./histogramBundles";
import {
  baseAxisLabel,
  baseCartesianGrid,
  buildErrorBarSeries,
  formatAxisValue,
} from "./chartPrimitives";
import LazyChart from "./LazyChart";
import Image2dPanel from "./Image2dPanel";

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
    return descriptor?.kind === "table" && expandsToHistogram;
  });
  const payload = bundlePanel?.state?.payload;
  const histograms = payload?.histograms;
  const expandsToHistogram =
    payload?.expands_to?.kind === "histogram" && payload?.expands_to?.source === "selected_row";
  if (
    bundlePanel &&
    expandsToHistogram &&
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
      renderablePanels.splice(renderablePanels.indexOf(bundlePanel) + 1, 0, {
        descriptor: {
          panel_id: `${sourcePanelId}_selected`,
          label: typeof payload?.expanded_label === "string" ? payload.expanded_label : "Selected Histogram",
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
          strata: payload.strata?.map((stratum, k) => ({ ...stratum, ...selectedHistogram.strata?.[k] })).filter((stratum) => Array.isArray(stratum.bins)),
        },
        value: bundlePanel.value ?? null,
      });
    }
  }

  const imageProgress = renderablePanels.find(({ descriptor, state }) => descriptor.panel_id === "image_progress" && state);
  const imageMode = renderablePanels.find(({ descriptor }) => descriptor.panel_id === "image_view_mode" && descriptor.kind === "select");
  if (imageProgress && imageMode) {
    imageProgress.controls = [imageMode];
    renderablePanels = renderablePanels.filter((panel) => panel !== imageMode);
  }
  return renderablePanels;
};

const timeseriesXAxisLabel = (mode) => {
  if (mode === HISTORY_X_AXIS_MODE_SAMPLER_UPTIME) return "Sampler Runner Uptime";
  if (mode === HISTORY_X_AXIS_MODE_COMPLETED_SAMPLES) return "Completed Samples";
  if (mode === HISTORY_X_AXIS_MODE_WALL_TIME) return "Elapsed Time";
  return "x";
};
const formatElapsedTime = (elapsedMs) => {
  const totalMs = Math.max(0, Math.round(Number(elapsedMs)));
  if (!Number.isFinite(totalMs)) return "n/a";
  const totalSeconds = Math.floor(totalMs / 1000);
  const fraction = totalMs % 1000 ? `.${String(totalMs % 1000).padStart(3, "0")}` : "";
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) {
    return `${String(hours).padStart(2, "0")}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}${fraction}`;
  }
  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}${fraction}`;
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
const formatActivityPercent = (value) => `${formatDecimalSignificant(value, 3)}%`;
const buildTimeseriesTooltipFormatter = (mode, isTimestamp, originMs, sources = [], formatValue = formatScientific) => (params) => {
  const seen = new Set();
  const entries = asArray(params).filter((entry) => {
    // ECharts axis tooltips can include silent custom series (range/error bars).
    if (entry.seriesType === "custom") return false;
    const key = entry.seriesId ?? entry.seriesName;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
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
    const label = `${entry?.marker ?? ""}${escapeXml(entry?.seriesName ?? "")}: ${rawValue != null && Number.isFinite(Number(rawValue)) ? formatValue(Number(rawValue)) : "n/a"}`;
    const source = sources.find((item) => item.label === entry.seriesName);
    const point = asArray(source?.points).findLast((p) => Number(p.x) === axisValue);
    if (!point?.estimator) return label;
    const error = point.error;
    const basic = point.basic ? `<br/>basic mean: ${formatScientific(point.basic.mean)}; basic error: ${formatScientific(point.basic.error)}` : "";
    return `${label}<br/>estimator: ${escapeXml(point.estimator)}; error: ${formatScientific(error)}${basic}`;
  });
  return [header, ...lines].join("<br/>");
};
const estimatorCaption = (points) => {
  const estimators = [...new Set(asArray(points).map((point) => point.estimator).filter(Boolean))];
  return estimators.length === 1 ? estimators[0] : estimators.length ? "mixed estimators" : null;
};
const estimateSeriesLabel = (label, points) => {
  const estimator = estimatorCaption(points);
  return estimator ? `${label} (${estimator})` : label;
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

const historyXAxisValueForPoint = (point, mode, nativeAxis) => {
  const raw = mode === nativeAxis
    ? point?.x
    : mode === HISTORY_X_AXIS_MODE_SAMPLER_UPTIME
      ? point?.x_sampler_uptime_ms
      : mode === HISTORY_X_AXIS_MODE_COMPLETED_SAMPLES
        ? point?.x_completed_samples_total
        : null;
  return raw != null && Number.isFinite(Number(raw)) ? Number(raw) : null;
};

const resolveTimeseriesXAxisMode = (value, nativeAxis, points) => {
  // Only performance histories offer alternate coordinates. Sample histories
  // and ordinary numeric plots keep their declared units, even with stale UI state.
  if (nativeAxis !== HISTORY_X_AXIS_MODE_WALL_TIME) return nativeAxis;
  const supportsMode = (mode) => points.some((point) => historyXAxisValueForPoint(point, mode, nativeAxis) != null);
  const defaultMode = supportsMode(HISTORY_X_AXIS_MODE_SAMPLER_UPTIME)
    ? HISTORY_X_AXIS_MODE_SAMPLER_UPTIME
    : nativeAxis;
  const requestedMode = readHistoryXAxisModeFromPanelValue(value, defaultMode);
  return supportsMode(requestedMode) ? requestedMode : nativeAxis;
};

const remapAndSortTimeseriesPoints = (points, mode, nativeAxis) =>
  asArray(points)
    .map((point) => ({ ...point, x: historyXAxisValueForPoint(point, mode, nativeAxis) }))
    // A missing alternate coordinate must never be replaced with a different unit.
    .filter((point) => point.x != null)
    .sort((left, right) => Number(left?.x) - Number(right?.x));

const lineColors = ["#00838f", "#bb3e03", "#0a9396", "#df5353", "#ca6702"];

const panelColumnSpan = (descriptor) => {
  if (descriptor?.panel_id === "campaign_progress") return "1 / -1";
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

const buildErrorBarData = (points) =>
  asArray(points)
    .map((point) => {
      if (point?.y_min == null || point?.y_max == null) return null;
      const x = Number(point?.x);
      const yMin = Number(point?.y_min);
      const yMax = Number(point?.y_max);
      if (!Number.isFinite(x) || !Number.isFinite(yMin) || !Number.isFinite(yMax) || yMax < yMin || yMax === yMin) {
        return null;
      }
      return [x, yMin, yMax];
    })
    .filter(Boolean);

const useTimeseriesLegend = (panelId, value, onValueChange) => {
  const [localSelection, setLocalSelection] = useState({});
  const selected = value?.legendSelected ?? localSelection;
  const onLegendChanged = useCallback((event) => {
    if (!event?.selected) return;
    setLocalSelection(event.selected);
    onValueChange?.(panelId, { ...value, legendSelected: event.selected }, false);
  }, [panelId, value, onValueChange]);
  return { selected, onLegendChanged };
};

const pointsInXRange = (points, range) => !range ? points : points.filter((point) =>
  Number.isFinite(Number(point.x)) && point.x >= range.min && point.x <= range.max);

const fitTimeseriesYDomain = (points, range, { mean = true, uncertainty = true, extraValues = [] } = {}) => {
  const inRange = pointsInXRange(points, range);
  return fitDomain([
    ...(inRange.length ? inRange : points).flatMap((point) => [
      ...(mean ? [point.y] : []),
      ...(uncertainty ? [point.y_min, point.y_max] : []),
    ]),
    ...extraValues,
  ]);
};

const ScalarTimeseriesPanel = ({ title, state, value = undefined, onValueChange = null }) => {
  const figureRef = useRef(null);
  const echartsRef = useRef(null);
  const panelId = state?.panel_id || null;
  const isHistoryPanel = useMemo(() => String(panelId || "").includes("_history"), [panelId]);
  const nativeAxis = state?.x_axis || "numeric";
  const historyXAxisMode = resolveTimeseriesXAxisMode(value, nativeAxis, asArray(state?.points));
  const points = remapAndSortTimeseriesPoints(state?.points, historyXAxisMode, nativeAxis);
  const estimatorLabel = estimatorCaption(points);
  const meanLabel = estimatorLabel ? `${title} (${estimatorLabel})` : title;
  const errorLabel = estimatorLabel ? `uncertainty (${estimatorLabel})` : "uncertainty";
  const { selected, onLegendChanged } = useTimeseriesLegend(panelId, value, onValueChange);
  const meanData = points.map((point) => [Number(point?.x), Number(point?.y)]);
  const targetValue = Number(state?.target);
  const hasTargetLine = state?.target != null && Number.isFinite(targetValue);
  const errorBarData = buildErrorBarData(points);
  const xDomain = fitXDomain(points.map((point) => point.x));
  const zoomRange = readZoomFromPanelValue(value, FULL_ZOOM);
  const usesTimestampXAxis = historyXAxisMode === HISTORY_X_AXIS_MODE_WALL_TIME;
  const xAxisOriginMs = useMemo(() => (usesTimestampXAxis ? Number(xDomain[0]) : 0), [usesTimestampXAxis, xDomain]);
  const tailPinned = readTailPinnedFromPanelValue(value, isHistoryPanel);
  const visibleXRange = useMemo(() => visibleXRangeFromZoom(xDomain, zoomRange), [xDomain, zoomRange]);
  const yZoomRange = readYZoomFromPanelValue(value, FULL_ZOOM);
  const visibleDomain = useMemo(() => fitTimeseriesYDomain(points, visibleXRange, {
    mean: selected[meanLabel || "Value"] !== false,
    uncertainty: selected[errorLabel] !== false,
    extraValues: hasTargetLine && selected.target !== false ? [targetValue] : [],
  }), [hasTargetLine, points, targetValue, visibleXRange, selected, meanLabel, errorLabel]);
  const bandSegments = useMemo(() => {
    const segments = [];
    for (let index = 1; index < points.length; index += 1) {
      const left = points[index - 1];
      const right = points[index];
      if (left?.y_min == null || left?.y_max == null || right?.y_min == null || right?.y_max == null) continue;
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
  const hasUncertaintyBand = isHistoryPanel && bandSegments.length > 0;
  const showLegend = hasUncertaintyBand || errorBarData.length > 0 || hasTargetLine || estimatorLabel != null;
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
      legendselectchanged: onLegendChanged,
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
    [isHistoryPanel, onValueChange, panelId, tailPinned, value, yZoomRange, zoomRange, onLegendChanged],
  );
  const option = useMemo(
    () => ({
      animation: false,
      legend: {
        selected,
        show: showLegend,
        top: 0,
        left: "center",
        textStyle: { fontSize: 12 },
      },
      grid: { ...baseCartesianGrid, left: 88, right: 56, top: showLegend ? 52 : 12, bottom: 64 },
      xAxis: {
        type: "value",
        min: xDomain[0],
        max: xDomain[1],
        name: timeseriesXAxisLabel(historyXAxisMode),
        nameLocation: "middle",
        nameGap: 30,
        axisLabel: {
          ...baseAxisLabel,
          formatter: (axisValue) =>
            formatTimeseriesXAxisValue(axisValue, historyXAxisMode, usesTimestampXAxis, xAxisOriginMs),
        },
        splitLine: { show: false },
        nameTextStyle: { fontSize: 12 },
      },
      yAxis: {
        type: "value",
        min: visibleDomain[0],
        max: visibleDomain[1],
        axisLabel: baseAxisLabel,
      },
      tooltip: {
        trigger: "axis",
        formatter: buildTimeseriesTooltipFormatter(historyXAxisMode, usesTimestampXAxis, xAxisOriginMs, [{ label: meanLabel, points }]),
      },
      dataZoom: buildDataZoom(zoomRange, true, true, yZoomRange, true),
      series: [
        ...(hasUncertaintyBand
          ? [
              {
                name: errorLabel,
                type: "custom",
                // A long-lived history can exceed ECharts' 3,000-item threshold.
                // Draw the band with the line, rather than 400 segments/frame.
                progressive: 0,
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
          : errorBarData.length > 0
            ? [buildErrorBarSeries({ name: errorLabel, data: errorBarData })]
            : []),
        {
          type: "line",
          name: meanLabel || "Value",
          data: meanData,
          smooth: Boolean(state?.smooth),
          showSymbol: false,
          lineStyle: { width: 1.8, color: "#00838f" },
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
      hasUncertaintyBand,
      meanData,
      hasTargetLine,
      showLegend,
      meanLabel,
      errorLabel,
      state?.smooth,
      targetValue,
      title,
      usesTimestampXAxis,
      visibleDomain,
      xAxisOriginMs,
      xDomain,
      yZoomRange,
      zoomRange,
      selected,
    ],
  );
  if (points.length === 0) return null;
  return (
    <Card variant="outlined">
      <CardContent>
        <PanelHeader title={title}>
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
        </PanelHeader>
        <Box ref={figureRef} sx={{ width: "100%", height: 280 }}>
          <LazyChart
            ref={echartsRef}
            option={option}
            notMerge={false}
            replaceMerge={["series"]}
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
  const percentAxis = panelId === "busy_history";
  const isHistoryPanel = useMemo(() => String(panelId || "").includes("_history"), [panelId]);
  const nativeAxis = state?.x_axis || "numeric";
  const historyXAxisMode = resolveTimeseriesXAxisMode(
    value,
    nativeAxis,
    asArray(state?.series).flatMap((item) => asArray(item?.points)),
  );
  const series = asArray(state?.series).map((item) => ({
    ...item,
    label: estimateSeriesLabel(item.label, item.points),
    points: remapAndSortTimeseriesPoints(item?.points, historyXAxisMode, nativeAxis),
  }));
  const { selected, onLegendChanged } = useTimeseriesLegend(panelId, value, onValueChange);
  const data = buildMultiSeriesData(series);
  const xDomain = state?.x_range ?? fitXDomain(data.map((row) => row.x));
  const zoomRange = useMemo(() => readZoomFromPanelValue(value, FULL_ZOOM), [value]);
  const yZoomRange = useMemo(() => readYZoomFromPanelValue(value, FULL_ZOOM), [value]);
  const visibleXRange = visibleXRangeFromZoom(xDomain, zoomRange);
  const visibleSeries = series.filter((item) => selected[item.label] !== false);
  // Explicit ranges (e.g. worker utilization at 0–100%) remain fixed.
  const domain = state?.y_range ?? fitTimeseriesYDomain(visibleSeries.flatMap((item) => item.points), visibleXRange);
  const tailPinned = readTailPinnedFromPanelValue(value, isHistoryPanel);
  const usesTimestampXAxis = historyXAxisMode === HISTORY_X_AXIS_MODE_WALL_TIME;
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
      legendselectchanged: onLegendChanged,
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
    [isHistoryPanel, onValueChange, panelId, tailPinned, value, yZoomRange, zoomRange, onLegendChanged],
  );
  const option = useMemo(
    () => ({
      animation: false,
      legend: {
        selected,
        top: 0,
        left: "center",
        textStyle: { fontSize: 12 },
      },
      grid: { ...baseCartesianGrid, left: 88, right: 56, top: 52, bottom: 64 },
      xAxis: {
        type: "value",
        min: xDomain[0],
        max: xDomain[1],
        name: timeseriesXAxisLabel(historyXAxisMode),
        nameLocation: "middle",
        nameGap: 30,
        axisLabel: {
          ...baseAxisLabel,
          formatter: (axisValue) =>
            formatTimeseriesXAxisValue(axisValue, historyXAxisMode, usesTimestampXAxis, xAxisOriginMs),
        },
        splitLine: { show: false },
        nameTextStyle: { fontSize: 12 },
      },
      yAxis: {
        type: "value",
        min: domain[0],
        max: domain[1],
        axisLabel: percentAxis ? { ...baseAxisLabel, formatter: formatActivityPercent } : baseAxisLabel,
      },
      tooltip: {
        trigger: "axis",
        formatter: buildTimeseriesTooltipFormatter(historyXAxisMode, usesTimestampXAxis, xAxisOriginMs, series, percentAxis ? formatActivityPercent : formatScientific),
      },
      dataZoom: buildDataZoom(zoomRange, true, true, yZoomRange, true),
      series: series.flatMap((item, index) => {
        const color = item.color || lineColors[index % lineColors.length];
        const lineSeries = {
          id: item.id,
          type: "line",
          name: item.label,
          data: asArray(item.points).flatMap((point) => {
            const sample = [Number(point?.x), Number(point?.y)];
            return point?.break_before ? [[sample[0], null], sample] : [sample];
          }),
          smooth: Boolean(item?.smooth),
          showSymbol: false,
          connectNulls: false,
          lineStyle: {
            width: 1.8,
            color,
          },
          itemStyle: {
            color,
          },
        };
        const errorBarData = buildErrorBarData(item.points);
        if (errorBarData.length === 0) return [lineSeries];
        return [buildErrorBarSeries({ name: item.label, data: errorBarData, color }), lineSeries];
      }),
    }),
    [domain, historyXAxisMode, panelId, percentAxis, series, usesTimestampXAxis, xAxisOriginMs, xDomain, yZoomRange, zoomRange, selected],
  );
  if (data.length === 0) return null;
  return (
    <Card variant="outlined">
      <CardContent>
        <PanelHeader title={title}>
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
        </PanelHeader>
        <Box ref={figureRef} sx={{ width: "100%", height: 280 }}>
          <LazyChart
            ref={echartsRef}
            option={option}
            notMerge={false}
            replaceMerge={["series"]}
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

const PanelRenderer = ({
  descriptor,
  state,
  value,
  controls = [],
  onValueChange,
  histogramBundlesByPanel,
  histogramBundleUploadErrors,
  histogramBundleUploads,
  uploadHistogramBundle,
  removeHistogramBundle,
  updateHistogramBundleSelection,
  removeComparedHistogram,
  addComparedHistogram,
  onSelectRun,
}) => {
  if (!descriptor) return null;
  switch (descriptor.kind) {
    case "strata":
      return state ? <StrataPanel title={descriptor.label} state={state} value={value} onValueChange={onValueChange}/> : null;
    case "pdf_comparison":
      return state ? <PdfComparisonPanel title={descriptor.label} state={state} value={value} onValueChange={onValueChange}/> : null;
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
      return <ProgressPanel title={descriptor.label} state={state} controls={controls.map((control) => (
        <Box key={control.descriptor.panel_id} sx={{ width: { xs: "100%", sm: 220 }, maxWidth: "100%" }}>
          <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 0.5 }}>{control.descriptor.label}</Typography>
          <SelectInput title={control.descriptor.label} descriptor={control.descriptor} value={control.value} onValueChange={onValueChange} />
        </Box>
      ))} />;
    case "key_value":
      if (!state) return null;
      return <KeyValuePanel title={descriptor.label} state={state} />;
    case "image2d":
      if (!state) return <EmptyPlotPanel title={descriptor.label} />;
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
          onSelectRun={onSelectRun}
          uploadedBundles={histogramBundlesByPanel?.[descriptor.panel_id] ?? []}
          bundleUploadError={histogramBundleUploadErrors?.[descriptor.panel_id] ?? null}
          bundleUploading={histogramBundleUploads?.[descriptor.panel_id] === true}
          onUploadBundle={uploadHistogramBundle}
          onRemoveBundle={removeHistogramBundle}
        />
      );
    case "histogram":
      if (!state) return <EmptyPlotPanel title={descriptor.label} />;
      return (
        <ObservableHistogramPanel
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

const PanelCollectionContent = ({
  title = null,
  panelSpecs,
  panelStates,
  panelValues = {},
  onPanelValueChange = null,
  onSelectRun = null,
  visiblePanelIds = null,
}) => {
  const {
    histogramBundlesByPanel,
    histogramBundleUploadErrors,
    histogramBundleUploads,
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
  const handlePanelValueChange = useCallback(
    (panelId, nextValue, shouldTriggerPoll = true) => {
      if (typeof onPanelValueChange !== "function") return;
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
    [onPanelValueChange, panelValues, sharedHistoryPanelIdSet, sharedHistoryPanelIds],
  );

  const displayedPanels = visiblePanelIds == null ? renderablePanels : renderablePanels.filter(({ descriptor, state }) =>
    visiblePanelIds.has(state?.source_panel_id || descriptor.panel_id));

  return (
    <Box sx={{ mb: displayedPanels.length === 1 && displayedPanels[0].descriptor.panel_id === "sample_progress" ? 1 : 3 }}>
      {title ? (
        <Typography variant="h6" sx={{ mb: 2 }}>
          {title}
        </Typography>
      ) : null}
      {displayedPanels.length === 0 && (visiblePanelIds == null || visiblePanelIds.size > 0) ? <Alert severity="info">No panel data available yet.</Alert> : null}
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
        {displayedPanels.map(({ descriptor, state, value, controls }) => (
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
              controls={controls}
              onValueChange={handlePanelValueChange}
              histogramBundlesByPanel={histogramBundlesByPanel}
              histogramBundleUploadErrors={histogramBundleUploadErrors}
              histogramBundleUploads={histogramBundleUploads}
              uploadHistogramBundle={uploadHistogramBundle}
              removeHistogramBundle={removeHistogramBundle}
              updateHistogramBundleSelection={updateHistogramBundleSelection}
              removeComparedHistogram={removeComparedHistogram}
              addComparedHistogram={addComparedHistogram}
              onSelectRun={onSelectRun}
            />
          </Box>
        ))}
      </Box>
    </Box>
  );
};

// Keep disclosures, tables and canvases mounted across tab switches. Hidden
// collections retain their last render; polling remains paused by the workspace.
const RetainedPanelCollection = memo(PanelCollectionContent, (_previous, next) => !next.active);
const PanelCollection = (props) => <RetainedPanelCollection {...props} active={useWorkspaceActive()} />;
export default PanelCollection;
