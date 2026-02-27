import { useState, useEffect, useRef } from "react";
import { fetchRuns } from "../services/api";

export const useRuns = (refreshInterval = 2000) => {
  const [runs, setRuns] = useState([]);
  const [selectedRun, setSelectedRun] = useState(null);
  const [isConnected, setIsConnected] = useState(false);
  const selectedRunRef = useRef(null);

  useEffect(() => {
    selectedRunRef.current = selectedRun;
  }, [selectedRun]);

  useEffect(() => {
    let cancelled = false;
    let timeoutId;
    let activeController = null;

    const loadRuns = async () => {
      activeController = new AbortController();
      try {
        const data = await fetchRuns(activeController.signal);
        if (cancelled) return;
        setRuns(data);
        setIsConnected(true);

        if (!selectedRunRef.current && data.length > 0) {
          setSelectedRun(data[0].run_id);
        }
      } catch (err) {
        if (err?.name === "AbortError" || cancelled) return;
        console.error("Failed to fetch runs:", err);
        setIsConnected(false);
        setRuns([]);
      } finally {
        activeController = null;
        if (!cancelled) {
          timeoutId = setTimeout(loadRuns, refreshInterval);
        }
      }
    };

    loadRuns();

    return () => {
      cancelled = true;
      if (timeoutId) clearTimeout(timeoutId);
      if (activeController) activeController.abort();
    };
  }, [refreshInterval]);

  return {
    runs,
    selectedRun,
    setSelectedRun,
    isConnected,
  };
};
