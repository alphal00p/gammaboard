import { useCallback } from "react";
import { fetchNodeLaunchRequests } from "../services/api";
import { usePolledResource } from "./usePolledResource";

export const useNodeLaunchRequests = ({ enabled = true, pollMs = 3000 } = {}) => {
  const fetchResource = useCallback((signal) => fetchNodeLaunchRequests(signal), []);
  const { data, isConnected, lastUpdate, error } = usePolledResource({
    enabled,
    pollMs,
    initialData: [],
    fetchResource,
  });

  return { launchRequests: data, isConnected, lastUpdate, error };
};
