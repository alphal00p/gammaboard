import { useState, useEffect } from "react";
import { fetchSamples, fetchStats } from "../services/api";
import { parseSamples } from "../utils/sampleParser";

/**
 * Custom hook for fetching and managing run data (samples and stats)
 * @param {number|null} runId - The run ID to fetch data for
 * @param {number} refreshInterval - Interval in milliseconds to refresh data (default: 1000)
 * @returns {Object} { samples, stats, lastUpdate }
 */
export const useRunData = (runId, refreshInterval = 1000) => {
  const [samples, setSamples] = useState([]);
  const [stats, setStats] = useState([]);
  const [lastUpdate, setLastUpdate] = useState(null);

  useEffect(() => {
    if (!runId) return;

    const loadData = async () => {
      try {
        // Fetch samples
        const samplesData = await fetchSamples(runId, 500);
        const parsedSamples = parseSamples(samplesData);
        setSamples(parsedSamples);

        // Fetch stats
        const statsData = await fetchStats(runId);
        setStats(statsData);

        setLastUpdate(new Date().toLocaleTimeString());
      } catch (err) {
        console.error("Failed to fetch data:", err);
      }
    };

    loadData();
    const interval = setInterval(loadData, refreshInterval);

    return () => clearInterval(interval);
  }, [runId, refreshInterval]);

  return {
    samples,
    stats,
    lastUpdate,
  };
};
