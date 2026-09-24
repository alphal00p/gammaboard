import { Alert, Button, Stack, Typography } from "@mui/material";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { fetchRunPerformanceGraphs } from "../services/api";
import PanelCollection from "./panels/PanelCollection";
import { readZoomFromPanelValue } from "./panels/panelView";

const IDS = ["busy_history", "accepted_rate_history"];
const seconds = (value) => `${Number(value).toLocaleString(undefined, { maximumFractionDigits: 3 })} s`;

export const selectedBounds = (selection, bounds) => {
  if (!selection || !bounds) return bounds;
  const end = selection.follow ? bounds[1] : selection.end;
  const start = selection.follow ? end - (selection.end - selection.start) : selection.start;
  return [Math.max(bounds[0], start), Math.min(bounds[1], end)];
};

// Keep coarse context outside the selected interval for the navigation slider;
// replace the visible interval with newly aggregated, finer-resolution data.
export const mergeHistoryDetail = (full, detail) => full.states.map((state) => {
  const focused = detail?.states.find((entry) => entry.panel_id === state.panel_id);
  if (!focused) return { ...state, x_range: full.bounds };
  const [start, end] = detail.selection;
  return { ...state, x_range: full.bounds, series: state.series.map((series) => {
    const points = focused.series.find((entry) => entry.id === series.id)?.points ?? [];
    const before = series.points.filter((point) => point.x < start);
    const after = series.points.filter((point) => point.x > end);
    return { ...series, points: [
      ...before,
      ...points.map((point, index) => index === 0 ? { ...point, break_before: true } : point),
      ...after.map((point, index) => index === 0 ? { ...point, break_before: true } : point),
    ] };
  }) };
});

const PerformanceGraphs = ({ runId }) => {
  const [selection, setSelection] = useState(null);
  const [yViews, setYViews] = useState({});
  const [data, setData] = useState(null);
  const [error, setError] = useState(null);
  const [loading, setLoading] = useState(true);
  const cache = useRef(null);
  const boundsRef = useRef(null);

  useEffect(() => {
    const controller = new AbortController();
    let timer;
    const load = async () => {
      setLoading(true);
      try {
        // Full history is only a bounded navigation overview. Refresh it less
        // often while inspecting a fixed past interval.
        let full = cache.current?.data;
        if (!full || !selection || selection.follow || Date.now() - cache.current.at > 30000) {
          full = await fetchRunPerformanceGraphs(runId, null, controller.signal);
          if (controller.signal.aborted) return;
          cache.current = { data: full, at: Date.now() };
        }
        const range = selectedBounds(selection, full.bounds);
        const detail = selection && range?.[0] < range?.[1]
          ? await fetchRunPerformanceGraphs(runId, range, controller.signal) : null;
        if (controller.signal.aborted) return;
        boundsRef.current = full.bounds;
        setData({ full, detail });
        setError(null);
      } catch (err) {
        if (!controller.signal.aborted) setError(err.message || "History could not be refreshed.");
      } finally {
        if (!controller.signal.aborted) {
          setLoading(false);
          timer = setTimeout(load, 5000);
        }
      }
    };
    // Debounce navigation; retain the graphs while the new detail loads.
    timer = setTimeout(load, selection ? 150 : 0);
    return () => { controller.abort(); clearTimeout(timer); };
  }, [runId, selection]);

  const changeView = useCallback((id, value) => {
    if (value?.yZoom) setYViews((previous) => ({ ...previous, [id]: value.yZoom }));
    const bounds = boundsRef.current;
    if (!bounds) return;
    const zoom = readZoomFromPanelValue(value);
    if (zoom.start <= 0 && zoom.end >= 100) { setSelection(null); return; }
    const width = bounds[1] - bounds[0];
    const start = Math.round(bounds[0] + width * zoom.start / 100);
    const end = Math.round(bounds[0] + width * zoom.end / 100);
    if (end <= start) return;
    const follow = zoom.end >= 99.999;
    setSelection((previous) => previous?.start === start && previous?.end === end && previous?.follow === follow
      ? previous : { start, end, follow });
  }, []);

  const full = data?.full;
  const measured = data?.detail ?? full;
  const states = useMemo(() => full ? mergeHistoryDetail(full, data.detail) : [], [full, data]);
  const range = selectedBounds(selection, full?.bounds);
  const zoom = range && full.bounds ? {
    start: 100 * (range[0] - full.bounds[0]) / (full.bounds[1] - full.bounds[0]),
    end: 100 * (range[1] - full.bounds[0]) / (full.bounds[1] - full.bounds[0]),
  } : { start: 0, end: 100 };
  const values = Object.fromEntries(IDS.map((id) => [id, { zoom, tailPinned: false, yZoom: yViews[id] ?? { start: 0, end: 100 } }]));

  return <Stack spacing={2}>
    <Stack direction="row" spacing={2} alignItems="center" flexWrap="wrap">
      <Button size="small" onClick={() => setSelection(null)} disabled={!selection}>Full history</Button>
      <Typography variant="body2" color="text.secondary">
        {selection ? (selection.follow ? "Following latest" : "Selected interval") : "All recorded history"}
        {loading ? " · Updating…" : ""}
      </Typography>
    </Stack>
    <Typography variant="body2" color="text.secondary">
      Drag the time slider handles to zoom; drag its selection to pan. Both graphs share the range.
      Select the right edge to follow new data. Zooming loads finer detail.
    </Typography>
    {error && <Alert severity="warning">{error} Displaying the last loaded history.</Alert>}
    {full?.bounds ? <>
      <PanelCollection panelSpecs={full.panels} panelStates={states} panelValues={values} onPanelValueChange={changeView} />
      <Typography variant="body2" color="text.secondary">
        Display bins: {seconds(measured.bin_seconds)}. Observed report spacing: {measured.cadence.map((cadence, index) =>
          `${index === 0 ? "evaluators" : "sampler"} ${cadence.intervals ? `${seconds(cadence.total_seconds / cadence.intervals)} average` : "unavailable"}`,
        ).join("; ")}.
      </Typography>
      <Typography variant="body2" color="text.secondary">
        Lines are interval averages. No smoothing is applied; gaps are unavailable.
        Historical averages cover reporting workers. Zoom in to resolve pauses.
        Overview and Diagnostics show the latest 60 seconds.
      </Typography>
    </> : !loading && <Alert severity="info">No recorded performance history yet.</Alert>}
  </Stack>;
};

export default PerformanceGraphs;
