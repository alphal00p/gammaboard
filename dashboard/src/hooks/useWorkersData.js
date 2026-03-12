import { useCallback, useState } from "react";
import { fetchNodes } from "../services/api";
import { usePolling } from "./usePolling";

const formatTime = () => new Date().toLocaleTimeString();

export const useWorkersData = ({ runId = null, pollMs = 3000 } = {}) => {
  const [workers, setWorkers] = useState([]);
  const [isConnected, setIsConnected] = useState(false);
  const [lastUpdate, setLastUpdate] = useState(null);
  const [error, setError] = useState(null);

  const poll = useCallback(
    async (signal) => {
      try {
        const data = await fetchNodes(runId, signal);
        setWorkers(Array.isArray(data) ? data : []);
        setError(null);
        setIsConnected(true);
        setLastUpdate(formatTime());
      } catch (err) {
        if (err?.name === "AbortError") return;
        setWorkers([]);
        setError(err);
        setIsConnected(false);
      }
    },
    [runId],
  );

  usePolling({ intervalMs: pollMs, poll });

  return { workers, isConnected, lastUpdate, error };
};
