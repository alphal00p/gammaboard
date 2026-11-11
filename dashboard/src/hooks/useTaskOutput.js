import { useCallback, useMemo } from "react";
import { fetchRunTaskPanels } from "../services/api";
import { mergePanelState, usePanelSource } from "./usePanelSource";
import { readResourceCache, writeResourceCache } from "../services/resourceCache";

export const useTaskOutput = ({ runId, taskId, section = "result", pollMs = 3000, panelLimit = 256, refreshKey, paused = false } = {}) => {
  const enabled = runId != null && taskId != null;
  const key = enabled ? `task:${runId}:${taskId}:${section}` : null;
  const fetchCurrent = useCallback(async ({ panelState, panelActions }, signal) => {
    const response = await fetchRunTaskPanels(runId, taskId, { limit: panelLimit, panelState, panelActions, section, phase: "current" }, signal);
    if (!signal.aborted && response?.available_sections) writeResourceCache(`task-meta:${runId}:${taskId}`, response.available_sections);
    return response;
  },
  [panelLimit, runId, taskId, section]);
  const current = usePanelSource({ enabled, pollMs, fetchPanels: fetchCurrent, useCursor: false, refreshKey, paused, cacheKey: key && `${key}:current` });
  const hasHistory = current.panelSpecs.some((spec) => spec.history === "append" || spec.history === "replace");
  const fetchHistory = useCallback(({ cursor }, signal) =>
    fetchRunTaskPanels(runId, taskId, { limit: panelLimit, cursor, section, phase: "history" }, signal),
  [panelLimit, runId, taskId, section]);
  const history = usePanelSource({ enabled: enabled && hasHistory, pollMs, fetchPanels: fetchHistory, refreshKey,
    paused, cacheKey: key && `${key}:history`, persistValues: false });
  const panelStates = useMemo(() => {
    if (!hasHistory) return current.panelStates;
    const states = new Map(history.panelStates.map((state) => [state.panel_id, state]));
    current.panelStates.forEach((state) => states.set(state.panel_id, states.has(state.panel_id) ? mergePanelState(states.get(state.panel_id), state) : state));
    return [...states.values()];
  }, [current.panelStates, history.panelStates, hasHistory]);
  return { ...current, panelStates, error: current.error || history.error,
    availableSections: current.availableSections ?? readResourceCache(`task-meta:${runId}:${taskId}`),
    isHistoryLoading: enabled && hasHistory && history.isLoading,
    isRevalidating: current.isRevalidating || (hasHistory && history.isRevalidating),
  };
};
