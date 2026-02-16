import { useState, useEffect } from "react";
import { fetchRuns } from "../services/api";

export const useRuns = (refreshInterval = 2000) => {
  const [runs, setRuns] = useState([]);
  const [selectedRun, setSelectedRun] = useState(null);
  const [isConnected, setIsConnected] = useState(false);

  useEffect(() => {
    const loadRuns = async () => {
      try {
        const data = await fetchRuns();
        setRuns(data);
        setIsConnected(true);

        if (!selectedRun && data.length > 0) {
          setSelectedRun(data[0].run_id);
        }
      } catch (err) {
        console.error("Failed to fetch runs:", err);
        setIsConnected(false);
        setRuns([]);
      }
    };

    loadRuns();
    const interval = setInterval(loadRuns, refreshInterval);

    return () => clearInterval(interval);
  }, [selectedRun, refreshInterval]);

  return {
    runs,
    selectedRun,
    setSelectedRun,
    isConnected,
  };
};
