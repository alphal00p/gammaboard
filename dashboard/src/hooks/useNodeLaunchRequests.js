import { fetchNodeLaunchRequests } from "../services/api";
import { usePolledResource } from "./usePolledResource";

export const useNodeLaunchRequests = ({ enabled = true, pollMs = 3000 } = {}) => {
  const { data, isConnected, lastUpdate, error } = usePolledResource({
    enabled,
    pollMs,
    initialData: [],
    fetchResource: fetchNodeLaunchRequests,
  });

  return { launchRequests: data, isConnected, lastUpdate, error };
};
