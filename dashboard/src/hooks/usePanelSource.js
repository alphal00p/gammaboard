import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { usePolling } from "./usePolling";
import { asArray } from "../utils/collections";

const emptyState = Object.freeze({
  sourceId: null,
  panelSpecs: [],
  panelStates: [],
  panelValues: {},
  cursor: null,
  error: null,
});

const asObject = (value) => (value && typeof value === "object" && !Array.isArray(value) ? value : {});

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
    : new Map(
        asArray(previousStates)
          .map((panel) => [panelIdOf(panel), panel])
          .filter(([id]) => id),
      );

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

const defaultPanelValue = (spec) => {
  const state = spec?.state;
  if (!state || typeof state !== "object") return undefined;
  if (state.kind === "select") return state.default_value ?? null;
  return undefined;
};

const reconcilePanelValues = (previousValues, panelSpecs, resetRequired) => {
  const next = resetRequired ? {} : { ...asObject(previousValues) };
  const knownIds = new Set();

  for (const spec of asArray(panelSpecs)) {
    if (!spec?.panel_id) continue;
    knownIds.add(spec.panel_id);
    if (!(spec.panel_id in next)) {
      const defaultValue = defaultPanelValue(spec);
      if (defaultValue !== undefined) next[spec.panel_id] = defaultValue;
    }
  }

  for (const key of Object.keys(next)) {
    if (!knownIds.has(key)) delete next[key];
  }

  return next;
};

export const usePanelSource = ({ enabled = true, pollMs = 5000, fetchPanels, useCursor = true } = {}) => {
  const [state, setState] = useState(emptyState);
  const cursorRef = useRef(null);
  const panelValuesRef = useRef({});
  const pendingActionsRef = useRef([]);

  useEffect(() => {
    cursorRef.current = state.cursor;
  }, [state.cursor]);

  useEffect(() => {
    panelValuesRef.current = state.panelValues;
  }, [state.panelValues]);

  const poll = useCallback(
    async (signal) => {
      if (!enabled || typeof fetchPanels !== "function") return;
      try {
        const response = await fetchPanels(
          {
            cursor: useCursor ? cursorRef.current : null,
            panelState: panelValuesRef.current,
            panelActions: pendingActionsRef.current,
          },
          signal,
        );
        pendingActionsRef.current = [];

        setState((previous) => {
          const resetRequired =
            response?.reset_required === true ||
            (previous.sourceId != null && response?.source_id != null && previous.sourceId !== response.source_id);
          const panelSpecs = asArray(response?.panels);
          const panelValues = reconcilePanelValues(previous.panelValues, panelSpecs, resetRequired);
          return {
            sourceId: response?.source_id ?? previous.sourceId,
            panelSpecs,
            panelStates: applyUpdates(previous.panelStates, response?.updates, resetRequired),
            panelValues,
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
    panelValuesRef.current = {};
    pendingActionsRef.current = [];
    setState(emptyState);
  }, []);

  const setPanelValue = useCallback((panelId, value) => {
    setState((previous) => {
      const panelValues = {
        ...asObject(previous.panelValues),
        [panelId]: value,
      };
      panelValuesRef.current = panelValues;
      cursorRef.current = null;
      return {
        ...previous,
        panelValues,
        cursor: null,
        panelStates: [],
      };
    });
  }, []);

  const invokePanelAction = useCallback((panelId, actionId, payload = null) => {
    pendingActionsRef.current = [
      ...pendingActionsRef.current,
      {
        panel_id: panelId,
        action_id: actionId,
        payload,
      },
    ];
  }, []);

  usePolling({ enabled, intervalMs: pollMs, poll, reset });

  return useMemo(
    () => ({
      ...state,
      setPanelValue,
      invokePanelAction,
    }),
    [invokePanelAction, setPanelValue, state],
  );
};
