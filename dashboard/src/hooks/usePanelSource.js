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

const arraysEqual = (left, right) => {
  if (left === right) return true;
  if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) return false;
  for (let index = 0; index < left.length; index += 1) {
    if (!Object.is(left[index], right[index])) return false;
  }
  return true;
};

const panelStateEquals = (left, right) => {
  if (left === right) return true;
  if (!isPlainObject(left) || !isPlainObject(right)) return false;
  if (left.kind !== right.kind || panelIdOf(left) !== panelIdOf(right)) return false;
  if (left.kind === "image2d" && right.kind === "image2d") {
    return (
      left.width === right.width &&
      left.height === right.height &&
      left.color_mode === right.color_mode &&
      left.normalization_mode === right.normalization_mode &&
      arraysEqual(left.x_range, right.x_range) &&
      arraysEqual(left.y_range, right.y_range) &&
      arraysEqual(left.values, right.values) &&
      arraysEqual(left.imag_values, right.imag_values) &&
      arraysEqual(left.invalid_indices, right.invalid_indices) &&
      left.metric_label === right.metric_label &&
      left.metric_mode === right.metric_mode &&
      left.x_label === right.x_label &&
      left.y_label === right.y_label
    );
  }
  if (left.kind === "progress" && right.kind === "progress") {
    return (
      left.current === right.current &&
      left.total === right.total &&
      left.unit === right.unit &&
      left.eta_seconds === right.eta_seconds
    );
  }
  return false;
};

const panelStateArraysEqual = (left, right) => {
  if (left === right) return true;
  if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) return false;
  for (let index = 0; index < left.length; index += 1) {
    if (!panelStateEquals(left[index], right[index])) return false;
  }
  return true;
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
      const previousPanel = next.get(panelId);
      next.set(panelId, panelStateEquals(previousPanel, panel) ? previousPanel : panel);
    }
  }

  const nextStates = Array.from(next.values());
  return panelStateArraysEqual(previousStates, nextStates) ? previousStates : nextStates;
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
  const refreshSeededPanelValuesRef = useRef(false);

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
        const nextPollAfterMs =
          response && Object.prototype.hasOwnProperty.call(response, "poll_after_ms")
            ? response.poll_after_ms
            : undefined;

        setState((previous) => {
          const nextSourceId = response?.source_id ?? previous.sourceId;
          const sourceChanged =
            previous.sourceId != null && response?.source_id != null && previous.sourceId !== response.source_id;
          const shouldSeedFromStorage = sourceChanged || (previous.sourceId == null && nextSourceId != null);
          const resetRequired = response?.reset_required === true || sourceChanged;
          const incomingPanelSpecs = asArray(response?.panels);
          const panelSpecs = incomingPanelSpecs.length > 0 ? incomingPanelSpecs : previous.panelSpecs;
          const seededPanelValues = shouldSeedFromStorage
            ? readStoredPanelValues(nextSourceId)
            : panelValuesRef.current;
          const panelValues = reconcilePanelValues(seededPanelValues, panelSpecs, resetRequired && !sourceChanged);
          if (shouldSeedFromStorage && Object.keys(panelValues).length > 0) {
            refreshSeededPanelValuesRef.current = true;
          }
          panelValuesRef.current = panelValues;
          const panelStates = applyUpdates(previous.panelStates, response?.updates, resetRequired);
          const cursor = response?.cursor ?? previous.cursor;
          if (
            previous.sourceId === nextSourceId &&
            previous.panelSpecs === panelSpecs &&
            previous.panelStates === panelStates &&
            previous.panelValues === panelValues &&
            previous.cursor === cursor &&
            previous.error == null
          ) {
            return previous;
          }
          return {
            sourceId: nextSourceId,
            panelSpecs,
            panelStates,
            panelValues,
            cursor,
            error: null,
          };
        });
        if (refreshSeededPanelValuesRef.current) {
          refreshSeededPanelValuesRef.current = false;
          return 1;
        }
        if (nextPollAfterMs === null) return null;
        if (Number.isFinite(Number(nextPollAfterMs)) && Number(nextPollAfterMs) > 0) return Number(nextPollAfterMs);
        return undefined;
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
      const nextPanelValues = {
        ...asObject(panelValuesRef.current),
        [panelId]: value,
      };
      panelValuesRef.current = nextPanelValues;
      setState((previous) => {
        return {
          ...previous,
          panelValues: nextPanelValues,
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
