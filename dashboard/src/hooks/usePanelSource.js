import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { usePolling } from "./usePolling";
import { asArray, isPlainObject } from "../utils/collections";

const emptyState = Object.freeze({
  sourceId: null,
  panelSpecs: [],
  panelStates: [],
  panelValues: {},
  cursor: null,
  error: null,
});

const asObject = (value) => (isPlainObject(value) ? value : {});
const PANEL_VALUES_STORAGE_PREFIX = "gammaboard.panel-values.";
const canUseStorage = () => typeof window !== "undefined" && typeof window.localStorage !== "undefined";
const storageKeyForSource = (sourceId) => `${PANEL_VALUES_STORAGE_PREFIX}${sourceId}`;
const readStoredPanelValues = (sourceId) => {
  if (!sourceId || !canUseStorage()) return {};
  try {
    const raw = window.localStorage.getItem(storageKeyForSource(sourceId));
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return asObject(parsed);
  } catch {
    return {};
  }
};
const writeStoredPanelValues = (sourceId, panelValues) => {
  if (!sourceId || !canUseStorage()) return;
  try {
    window.localStorage.setItem(storageKeyForSource(sourceId), JSON.stringify(asObject(panelValues)));
  } catch {
    // Ignore storage write errors to avoid breaking panel rendering in constrained browsers.
  }
};

const panelIdOf = (panel) => panel?.panel_id ?? null;

const mergePlotPoints = (previousPoints, incomingPoints) => {
  const byX = new Map();
  for (const point of asArray(previousPoints)) {
    const x = Number(point?.x);
    if (!Number.isFinite(x)) continue;
    byX.set(x, point);
  }
  for (const point of asArray(incomingPoints)) {
    const x = Number(point?.x);
    if (!Number.isFinite(x)) continue;
    byX.set(x, point);
  }
  return Array.from(byX.entries())
    .sort((left, right) => left[0] - right[0])
    .map(([, point]) => point);
};

const mergePanelState = (previous, incoming) => {
  if (!previous) return incoming;
  if (previous.kind === "scalar_timeseries" && incoming.kind === "scalar_timeseries") {
    return {
      ...previous,
      points: mergePlotPoints(previous.points, incoming.points),
    };
  }
  if (previous.kind === "multi_timeseries" && incoming.kind === "multi_timeseries") {
    const seriesMap = new Map(asArray(previous.series).map((series) => [series.id, { ...series }]));
    for (const series of asArray(incoming.series)) {
      const existing = seriesMap.get(series.id) || { ...series, points: [] };
      existing.points = mergePlotPoints(existing.points, series.points);
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
  if (!resetRequired && asArray(updates).length === 0) return previousStates;
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
  const previous = asObject(previousValues);
  const next = resetRequired ? {} : { ...previous };
  let changed = resetRequired && Object.keys(previous).length > 0;
  const knownIds = new Set();

  for (const spec of asArray(panelSpecs)) {
    if (!spec?.panel_id) continue;
    knownIds.add(spec.panel_id);
    if (!(spec.panel_id in next)) {
      const defaultValue = defaultPanelValue(spec);
      if (defaultValue !== undefined) {
        next[spec.panel_id] = defaultValue;
        changed = true;
      }
    }
  }

  for (const key of Object.keys(next)) {
    if (!knownIds.has(key)) {
      delete next[key];
      changed = true;
    }
  }

  return changed ? next : previous;
};

const panelValueEquals = (left, right) => {
  if (Object.is(left, right)) return true;
  if (!isPlainObject(left) || !isPlainObject(right)) return false;
  const leftEntries = Object.entries(left);
  const rightEntries = Object.entries(right);
  if (leftEntries.length !== rightEntries.length) return false;
  return leftEntries.every(([key, value]) => Object.is(value, right[key]));
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

  useEffect(() => {
    writeStoredPanelValues(state.sourceId, state.panelValues);
  }, [state.panelValues, state.sourceId]);

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
          const nextSourceId = response?.source_id ?? previous.sourceId;
          const sourceChanged =
            previous.sourceId != null && response?.source_id != null && previous.sourceId !== response.source_id;
          const shouldSeedFromStorage = sourceChanged || (previous.sourceId == null && nextSourceId != null);
          const resetRequired = response?.reset_required === true || sourceChanged;
          const panelSpecs = asArray(response?.panels);
          const seededPanelValues = shouldSeedFromStorage
            ? readStoredPanelValues(nextSourceId)
            : panelValuesRef.current;
          const panelValues = reconcilePanelValues(seededPanelValues, panelSpecs, resetRequired && !sourceChanged);
          panelValuesRef.current = panelValues;
          return {
            sourceId: nextSourceId,
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

  const triggerPoll = usePolling({ enabled, intervalMs: pollMs, poll, reset });

  const setPanelValue = useCallback(
    (panelId, value, shouldTriggerPoll = true) => {
      const existing = panelValuesRef.current?.[panelId];
      if (panelValueEquals(existing, value)) return;
      setState((previous) => {
        const panelValues = {
          ...asObject(previous.panelValues),
          [panelId]: value,
        };
        panelValuesRef.current = panelValues;
        return {
          ...previous,
          panelValues,
        };
      });
      if (shouldTriggerPoll) triggerPoll();
    },
    [triggerPoll],
  );

  const invokePanelAction = useCallback(
    (panelId, actionId, payload = null) => {
      pendingActionsRef.current = [
        ...pendingActionsRef.current,
        {
          panel_id: panelId,
          action_id: actionId,
          payload,
        },
      ];
      triggerPoll();
    },
    [triggerPoll],
  );

  return useMemo(
    () => ({
      ...state,
      setPanelValue,
      invokePanelAction,
    }),
    [invokePanelAction, setPanelValue, state],
  );
};
