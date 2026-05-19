import { useCallback } from "react";
import { fetchServerStatus } from "../services/api";
import { usePolledResource } from "./usePolledResource";

export const useServerStatus = (pollMs = 3000) => {
  const fetchResource = useCallback((signal) => fetchServerStatus(signal), []);
  const { data, isConnected, lastUpdate, error } = usePolledResource({
    pollMs,
    initialData: { server_name: "local", status: "unknown", database: "unknown" },
    fetchResource,
  });

  return {
    serverName: typeof data?.server_name === "string" && data.server_name.trim() ? data.server_name.trim() : "local",
    isConnected,
    lastUpdate,
    error,
  };
};
