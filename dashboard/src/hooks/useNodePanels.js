import { useCallback } from "react";
import { fetchNodePanels } from "../services/api";
import { usePanelSource } from "./usePanelSource";

export const useNodePanels = ({ nodeName, pollMs = 3000 } = {}) => {
  const enabled = nodeName != null;

  const fetchPanels = useCallback(
    (_request, signal) => {
      if (!enabled) return null;
      return fetchNodePanels(nodeName, signal);
    },
    [enabled, nodeName],
  );

  return usePanelSource({
    enabled,
    pollMs,
    fetchPanels,
    useCursor: false,
  });
};
