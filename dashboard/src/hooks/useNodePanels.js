import { useCallback } from "react";
import { fetchNodePanels } from "../services/api";
import { usePanelSource } from "./usePanelSource";

export const useNodePanels = ({ nodeId, pollMs = 3000 } = {}) => {
  const enabled = nodeId != null;

  const fetchPanels = useCallback(
    (_request, signal) => {
      if (!enabled) return null;
      return fetchNodePanels(nodeId, signal);
    },
    [enabled, nodeId],
  );

  return usePanelSource({
    enabled,
    pollMs,
    fetchPanels,
    useCursor: false,
  });
};
