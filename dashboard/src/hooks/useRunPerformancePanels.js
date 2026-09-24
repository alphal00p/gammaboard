import { useCallback } from "react";
import { fetchRunPerformance } from "../services/api";
import { usePanelSource } from "./usePanelSource";

export const useRunPerformancePanels = ({ runId, evaluatorNodeName = null, windowSeconds = 60, pollMs = 5000 } = {}) => {
  const fetchPanels = useCallback(
    (_request, signal) => fetchRunPerformance(runId, windowSeconds, evaluatorNodeName, signal),
    [evaluatorNodeName, windowSeconds, runId],
  );
  return usePanelSource({
    enabled: runId != null,
    pollMs,
    fetchPanels,
    useCursor: false,
  });
};
