import { useCallback } from "react";
import { fetchRunTasks } from "../services/api";
import { usePolledResource } from "./usePolledResource";

export const useRunTasks = (runId, refreshInterval = 2000) => {
  const fetchResource = useCallback((signal) => (runId == null ? [] : fetchRunTasks(runId, signal)), [runId]);
  const { data } = usePolledResource({
    enabled: runId != null,
    pollMs: refreshInterval,
    initialData: [],
    fetchResource,
    onError: (err) => console.error("Failed to fetch run tasks:", err),
  });

  return { tasks: data };
};
