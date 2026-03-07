import { useCallback, useState } from "react";
import { fetchRuns } from "../services/api";
import { usePolling } from "./usePolling";

export const useRuns = (refreshInterval = 2000) => {
  const [runs, setRuns] = useState([]);
  const [isConnected, setIsConnected] = useState(false);

  const poll = useCallback(async (signal) => {
    try {
      const data = await fetchRuns(signal);
      setRuns(data);
      setIsConnected(true);
    } catch (err) {
      if (err?.name === "AbortError") return;
      console.error("Failed to fetch runs:", err);
      setIsConnected(false);
      setRuns([]);
    }
  }, []);

  usePolling({ intervalMs: refreshInterval, poll });

  return {
    runs,
    isConnected,
  };
};
