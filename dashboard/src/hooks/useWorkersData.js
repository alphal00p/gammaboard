import { useCallback } from "react";
import { fetchNodes } from "../services/api";
import { asArray } from "../utils/collections";
import { usePolledResource } from "./usePolledResource";

export const useWorkersData = ({ runId = null, pollMs = 3000 } = {}) => {
  const fetchResource = useCallback((signal) => fetchNodes(runId, signal).then(asArray), [runId]);
  const { data, isConnected, lastUpdate, error } = usePolledResource({
    pollMs,
    initialData: [],
    fetchResource,
  });

  return { workers: data, isConnected, lastUpdate, error };
};
