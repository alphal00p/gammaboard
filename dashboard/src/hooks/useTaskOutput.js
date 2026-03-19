import { useCallback } from "react";
import { fetchRunTaskPanels } from "../services/api";
import { usePanelSource } from "./usePanelSource";

export const useTaskOutput = ({ runId, taskId, pollMs = 3000, historyLimit = 500 } = {}) => {
  const enabled = runId != null && taskId != null;

  const fetchPanels = useCallback(
    ({ afterCursor }, signal) => {
      if (!enabled) return null;
      return fetchRunTaskPanels(
        runId,
        taskId,
        {
          limit: historyLimit,
          afterCursor,
        },
        signal,
      );
    },
    [enabled, historyLimit, runId, taskId],
  );

  return usePanelSource({
    enabled,
    pollMs,
    fetchPanels,
    useCursor: true,
  });
};
