import { useCallback } from "react";
import { fetchRunConfigPanels } from "../services/api";
import { usePanelSource } from "./usePanelSource";

export const useRunConfigPanels = ({ runId, pollMs = 5000 } = {}) => {
  const enabled = runId != null;

  const fetchPanels = useCallback(
    (_request, signal) => {
      if (!enabled) return null;
      return fetchRunConfigPanels(runId, signal);
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
