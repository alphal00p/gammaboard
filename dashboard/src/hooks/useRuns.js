import { useState, useEffect } from "react";
import { fetchRuns } from "../services/api";

/**
 * Custom hook for fetching and managing the list of runs
 * @param {number} refreshInterval - Interval in milliseconds to refresh runs (default: 2000)
 * @returns {Object} { runs, selectedRun, setSelectedRun, isConnected }
 */
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

        // Auto-select first run if none selected
        if (!selectedRun && data.length > 0) {
          setSelectedRun(data[0].run_id);
        }
      } catch (err) {
        console.error("Failed to fetch runs:", err);
        setIsConnected(false);
        setRuns([]); // Clear runs on disconnect
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
