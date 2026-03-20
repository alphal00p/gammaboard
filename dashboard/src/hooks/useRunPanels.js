import { useCallback } from "react";
import { fetchRunPanels } from "../services/api";
import { usePanelSource } from "./usePanelSource";

export const useRunPanels = ({ runId, pollMs = 5000 } = {}) => {
  const enabled = runId != null;

  const fetchPanels = useCallback(
    (_, signal) => {
      if (!enabled) return null;
      return fetchRunPanels(runId, signal);
    },
    [enabled, runId],
  );

  return usePanelSource({
    enabled,
    pollMs,
    fetchPanels,
    useCursor: false,
  });
};
