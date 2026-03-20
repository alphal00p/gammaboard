import { useCallback } from "react";
import { fetchRuns } from "../services/api";
import { usePolledResource } from "./usePolledResource";

export const useRuns = (refreshInterval = 2000) => {
  const fetchResource = useCallback((signal) => fetchRuns(signal), []);
  const { data, isConnected } = usePolledResource({
    pollMs: refreshInterval,
    initialData: [],
    fetchResource,
    onError: (err) => console.error("Failed to fetch runs:", err),
  });

  return { runs: data, isConnected };
};
