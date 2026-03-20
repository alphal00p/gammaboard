import { useCallback } from "react";
import { fetchRunTaskPanels } from "../services/api";
import { usePanelSource } from "./usePanelSource";

export const useTaskOutput = ({ runId, taskId, pollMs = 3000, panelLimit = 500 } = {}) => {
  const enabled = runId != null && taskId != null;

  const fetchPanels = useCallback(
    ({ cursor, panelState, panelActions }, signal) => {
      if (!enabled) return null;
      return fetchRunTaskPanels(
        runId,
        taskId,
        {
          limit: panelLimit,
          cursor,
          panelState,
          panelActions,
        },
        signal,
      );
    },
    [enabled, panelLimit, runId, taskId],
  );

  return usePanelSource({
    enabled,
    pollMs,
    fetchPanels,
    useCursor: true,
  });
};
