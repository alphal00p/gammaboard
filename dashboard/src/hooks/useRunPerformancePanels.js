import { useCallback } from "react";
import { fetchRunPerformance } from "../services/api";
import { usePanelSource } from "./usePanelSource";

export const useRunPerformancePanels = ({ runId, evaluatorNodeName = null, selection = null, pollMs = 5000 } = {}) => {
  const fetchPanels = useCallback(
    (_request, signal) => fetchRunPerformance(runId, selection, evaluatorNodeName, signal),
    [evaluatorNodeName, selection, runId],
  );
  return usePanelSource({
    enabled: runId != null,
    pollMs,
    fetchPanels,
    useCursor: false,
  });
};
