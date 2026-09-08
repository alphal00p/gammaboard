import { useCallback } from "react";
import { fetchRunPerformance } from "../services/api";
import { usePanelSource } from "./usePanelSource";

export const useRunPerformancePanels = ({ runId, evaluatorNodeName = null, limit = 200, pollMs = 5000 } = {}) => {
  const fetchPanels = useCallback(
    (_request, signal) => fetchRunPerformance(runId, limit, evaluatorNodeName, signal),
    [evaluatorNodeName, limit, runId],
  );
  return usePanelSource({
    enabled: runId != null,
    pollMs,
    fetchPanels,
    useCursor: false,
  });
};
