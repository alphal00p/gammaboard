import { useCallback, useEffect, useRef, useState } from "react";
import { fetchRunPerformance } from "../services/api";
import { usePanelSource } from "./usePanelSource";

export const useRunPerformancePanels = ({ runId, evaluatorNodeName = null, selection = null, pollMs = 5000, paused = false } = {}) => {
  const [refreshing, setRefreshing] = useState(false);
  const parameters = useRef({ selection, evaluatorNodeName });
  useEffect(() => { parameters.current = { selection, evaluatorNodeName }; }, [selection, evaluatorNodeName]);
  // Window changes refresh this source without clearing its panels. Run changes
  // still reset the source, so another run's measurements are never retained.
  const fetchPanels = useCallback(
    async (_request, signal) => {
      setRefreshing(true);
      try {
        const result = await fetchRunPerformance(runId, parameters.current.selection, parameters.current.evaluatorNodeName, signal);
        return { ...result, poll_after_ms: pollMs };
      } finally {
        if (!signal.aborted) setRefreshing(false);
      }
    },
    [runId, pollMs],
  );
  const source = usePanelSource({
    enabled: runId != null,
    pollMs,
    fetchPanels,
    useCursor: false,
    paused,
    retainOnCacheChange: true,
    cacheKey: `performance:${runId}:${JSON.stringify(selection)}:${evaluatorNodeName ?? "all"}`,
  });
  return { ...source, refreshing: runId != null && refreshing };
};
