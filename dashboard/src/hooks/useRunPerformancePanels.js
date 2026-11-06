import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { fetchRunPerformance } from "../services/api";
import { usePanelSource } from "./usePanelSource";

export const useRunPerformancePanels = ({ runId, evaluatorNodeName = null, selection = null, pollMs = 5000 } = {}) => {
  const [refreshing, setRefreshing] = useState(false);
  const parameters = useRef({ selection, evaluatorNodeName });
  useEffect(() => { parameters.current = { selection, evaluatorNodeName }; }, [selection, evaluatorNodeName]);
  const refreshKey = useMemo(() => ({ selection, evaluatorNodeName }), [selection, evaluatorNodeName]);
  // Window changes refresh this source without clearing its panels. Run changes
  // still reset the source, so another run's measurements are never retained.
  const fetchPanels = useCallback(
    async (_request, signal) => {
      setRefreshing(true);
      try {
        return await fetchRunPerformance(runId, parameters.current.selection, parameters.current.evaluatorNodeName, signal);
      } finally {
        if (!signal.aborted) setRefreshing(false);
      }
    },
    [runId],
  );
  const source = usePanelSource({
    enabled: runId != null,
    pollMs,
    fetchPanels,
    useCursor: false,
    refreshKey,
  });
  return { ...source, refreshing: runId != null && refreshing };
};
