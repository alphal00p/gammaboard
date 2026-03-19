import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { usePolling } from "./usePolling";
import { asArray } from "../utils/collections";

const emptyState = Object.freeze({
  sourceId: null,
  panelSpecs: [],
  panelStates: [],
  cursor: null,
  error: null,
});

const panelIdOf = (panel) => panel?.panel_id ?? null;

const mergePanelState = (previous, incoming) => {
  if (!previous) return incoming;
  if (previous.kind === "scalar_timeseries" && incoming.kind === "scalar_timeseries") {
    return {
      ...previous,
      points: [...asArray(previous.points), ...asArray(incoming.points)],
    };
  }
  if (previous.kind === "multi_timeseries" && incoming.kind === "multi_timeseries") {
    const seriesMap = new Map(asArray(previous.series).map((series) => [series.id, { ...series }]));
    for (const series of asArray(incoming.series)) {
      const existing = seriesMap.get(series.id) || { ...series, points: [] };
      existing.points = [...asArray(existing.points), ...asArray(series.points)];
      seriesMap.set(series.id, existing);
    }
    return {
      ...previous,
      series: Array.from(seriesMap.values()),
    };
  }
  return incoming;
};

const applyUpdates = (previousStates, updates, resetRequired) => {
  const next = resetRequired
    ? new Map()
    : new Map(asArray(previousStates).map((panel) => [panelIdOf(panel), panel]).filter(([id]) => id));

  for (const update of asArray(updates)) {
    const panel = update?.panel;
    const panelId = panelIdOf(panel);
    if (!panelId) continue;
    if (update?.mode === "append") {
      next.set(panelId, mergePanelState(next.get(panelId), panel));
    } else {
      next.set(panelId, panel);
    }
  }

  return Array.from(next.values());
};

export const usePanelSource = ({
  enabled = true,
  pollMs = 5000,
  fetchPanels,
  useCursor = true,
} = {}) => {
  const [state, setState] = useState(emptyState);
  const cursorRef = useRef(null);

  useEffect(() => {
    cursorRef.current = state.cursor;
  }, [state.cursor]);

  const poll = useCallback(
    async (signal) => {
      if (!enabled || typeof fetchPanels !== "function") return;
      try {
        const response = await fetchPanels(
          {
            afterCursor: useCursor ? cursorRef.current : null,
          },
          signal,
        );

        setState((previous) => {
          const resetRequired =
            response?.reset_required === true ||
            (previous.sourceId != null && response?.source_id != null && previous.sourceId !== response.source_id);
          return {
            sourceId: response?.source_id ?? previous.sourceId,
            panelSpecs: asArray(response?.panels),
            panelStates: applyUpdates(previous.panelStates, response?.updates, resetRequired),
            cursor: response?.cursor ?? previous.cursor,
            error: null,
          };
        });
      } catch (err) {
        if (err?.name === "AbortError") return;
        setState((previous) => ({
          ...previous,
          error: err?.message || "Failed to fetch panels",
        }));
      }
    },
    [enabled, fetchPanels, useCursor],
  );

  const reset = useCallback(() => {
    cursorRef.current = null;
    setState(emptyState);
  }, []);

  usePolling({ enabled, intervalMs: pollMs, poll, reset });

  return useMemo(() => state, [state]);
};
