export const isObject = (value) => value && typeof value === "object" && !Array.isArray(value);

export const FULL_ZOOM = Object.freeze({ start: 0, end: 100 });

export const HISTORY_X_AXIS_MODE_WALL_TIME = "wall_time";
export const HISTORY_X_AXIS_MODE_SAMPLER_UPTIME = "sampler_uptime";
export const HISTORY_X_AXIS_MODE_COMPLETED_SAMPLES = "completed_samples";
export const HISTORY_X_AXIS_MODE_SET = new Set([
  HISTORY_X_AXIS_MODE_WALL_TIME,
  HISTORY_X_AXIS_MODE_SAMPLER_UPTIME,
  HISTORY_X_AXIS_MODE_COMPLETED_SAMPLES,
]);

export const isSharedHistoryTimeseriesPanelSpec = (spec) => {
  const kind = String(spec?.kind || "");
  const history = String(spec?.history || "");
  return (kind === "scalar_timeseries" || kind === "multi_timeseries") && history !== "none";
};

export const normalizeZoomRange = (candidate) => {
  const start = Number(candidate?.start);
  const end = Number(candidate?.end);
  if (!Number.isFinite(start) || !Number.isFinite(end)) return null;
  const normalizedStart = Math.max(0, Math.min(100, start));
  const normalizedEnd = Math.max(0, Math.min(100, end));
  if (normalizedEnd < normalizedStart) return { start: normalizedEnd, end: normalizedStart };
  return { start: normalizedStart, end: normalizedEnd };
};

export const readZoomFromPanelValue = (value, fallback = FULL_ZOOM) =>
  normalizeZoomRange(isObject(value) ? value.zoom : null) || fallback;

export const readYZoomFromPanelValue = (value, fallback = FULL_ZOOM) =>
  normalizeZoomRange(isObject(value) ? value.yZoom : null) || fallback;

export const readTailPinnedFromPanelValue = (value, fallback = true) => {
  if (typeof value?.tailPinned === "boolean") return value.tailPinned;
  const zoom = normalizeZoomRange(isObject(value) ? value.zoom : null);
  return zoom ? zoom.end >= 99.5 : fallback;
};

export const readHistoryXAxisModeFromPanelValue = (value, fallback = HISTORY_X_AXIS_MODE_WALL_TIME) => {
  const mode = isObject(value) ? value.xAxisMode : null;
  return HISTORY_X_AXIS_MODE_SET.has(mode) ? mode : fallback;
};

export const writeZoomPanelValue = (current, zoom, tailPinned = null, yZoom = undefined) => {
  const next = isObject(current) ? { ...current } : {};
  next.zoom = normalizeZoomRange(zoom) || FULL_ZOOM;
  if (yZoom !== undefined) next.yZoom = normalizeZoomRange(yZoom) || FULL_ZOOM;
  if (tailPinned != null) next.tailPinned = Boolean(tailPinned);
  return next;
};

export const extractSharedHistoryView = (value) => {
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

export const mergeSharedHistoryView = (current, sharedView) => {
  const next = isObject(current) ? { ...current } : {};
  if (sharedView.zoom) next.zoom = sharedView.zoom;
  if ("tailPinned" in sharedView) next.tailPinned = sharedView.tailPinned;
  if ("xAxisMode" in sharedView && HISTORY_X_AXIS_MODE_SET.has(sharedView.xAxisMode)) {
    next.xAxisMode = sharedView.xAxisMode;
  }
  return next;
};

export const extractSharedImageZoom = (value) => {
  if (!isObject(value)) return null;
  const zoom = normalizeZoomRange(value.zoom);
  return zoom ? { zoom } : null;
};

export const mergeSharedImageZoom = (current, sharedView) => {
  const next = isObject(current) ? { ...current } : {};
  if (sharedView.zoom) next.zoom = sharedView.zoom;
  return next;
};

export const zoomRangeChanged = (left, right) =>
  Math.abs((left?.start ?? 0) - (right?.start ?? 0)) > 0.01 ||
  Math.abs((left?.end ?? 100) - (right?.end ?? 100)) > 0.01;

export const readDataZoomRanges = (event) => {
  const payloads = Array.isArray(event?.batch) && event.batch.length > 0 ? event.batch : [event];
  const xPayload = payloads.find((payload) => {
    if (!isObject(payload)) return false;
    const dataZoomId = typeof payload.dataZoomId === "string" ? payload.dataZoomId : "";
    if (dataZoomId.startsWith("x-")) return true;
    if (Array.isArray(payload.xAxisIndex)) return payload.xAxisIndex.includes(0);
    if (Number.isFinite(Number(payload.xAxisIndex))) return Number(payload.xAxisIndex) === 0;
    return false;
  });
  const yPayload = payloads.find((payload) => {
    if (!isObject(payload)) return false;
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

export const visibleXRangeFromZoom = (xDomain, zoomRange) => {
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

export const visibleXRangeFromZoomWithScale = (xDomain, zoomRange, scale = "linear") => {
  const normalizedZoom = normalizeZoomRange(zoomRange);
  const xMin = Number(xDomain?.[0]);
  const xMax = Number(xDomain?.[1]);
  if (!normalizedZoom || !Number.isFinite(xMin) || !Number.isFinite(xMax)) return null;
  if (scale !== "log") return visibleXRangeFromZoom(xDomain, normalizedZoom);

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
};

export const buildDataZoom = (
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
      zoomOnMouseWheel: false,
      moveOnMouseWheel: false,
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
      zoomOnMouseWheel: false,
      moveOnMouseWheel: false,
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
