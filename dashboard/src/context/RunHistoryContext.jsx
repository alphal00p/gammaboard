import React, { createContext, useCallback, useContext, useMemo, useEffect, useState } from "react";
import {
  createRunStatsEventSource,
  fetchAggregatedHistory as fetchAggregatedHistoryApi,
  fetchLatestAggregated as fetchLatestAggregatedApi,
  fetchRun as fetchRunApi,
  fetchStats as fetchStatsApi,
} from "../services/api";

const RunHistoryContext = createContext(null);

const formatTime = () => new Date().toLocaleTimeString();

export const RunHistoryProvider = ({
  runId,
  children,
  historyLimit = 200,
  pollIntervalMs = 5000,
  streamIntervalMs = 1000,
}) => {
  const [history, setHistory] = useState([]);
  const [latestAggregated, setLatestAggregated] = useState(null);
  const [run, setRun] = useState(null);
  const [workQueueStats, setWorkQueueStats] = useState([]);
  const [isConnected, setIsConnected] = useState(false);
  const [lastUpdate, setLastUpdate] = useState(null);
  const [error, setError] = useState(null);

  const mergeLatest = useCallback(
    (next) => {
      if (!next) return;
      setHistory((prev) => {
        if (prev.some((item) => item.id === next.id)) return prev;
        const merged = [next, ...prev];
        return merged.slice(0, historyLimit);
      });
    },
    [historyLimit],
  );

  const fetchAggregatedHistory = useCallback(async () => {
    if (!runId) return;
    const data = await fetchAggregatedHistoryApi(runId, historyLimit);
    setHistory(data);
    setLatestAggregated(data[0] || null);
  }, [runId, historyLimit]);

  const fetchLatestAggregated = useCallback(async () => {
    if (!runId) return;
    const data = await fetchLatestAggregatedApi(runId);
    if (!data) {
      setLatestAggregated(null);
      return;
    }
    setLatestAggregated(data);
    mergeLatest(data);
  }, [runId, mergeLatest]);

  const fetchRun = useCallback(async () => {
    if (!runId) return;
    const data = await fetchRunApi(runId);
    setRun(data);
  }, [runId]);

  const fetchWorkQueueStats = useCallback(async () => {
    if (!runId) return;
    const data = await fetchStatsApi(runId);
    setWorkQueueStats(data);
  }, [runId]);

  useEffect(() => {
    if (!runId) {
      setHistory([]);
      setLatestAggregated(null);
      setRun(null);
      setWorkQueueStats([]);
      setIsConnected(false);
      setLastUpdate(null);
      setError(null);
      return;
    }

    let cancelled = false;

    const loadInitial = async () => {
      try {
        await Promise.all([fetchAggregatedHistory(), fetchRun(), fetchWorkQueueStats()]);
        if (!cancelled) {
          setError(null);
          setIsConnected(true);
          setLastUpdate(formatTime());
        }
      } catch (err) {
        if (!cancelled) {
          setError(err);
          setIsConnected(false);
        }
      }
    };

    loadInitial();

    return () => {
      cancelled = true;
    };
  }, [runId, fetchAggregatedHistory, fetchRun, fetchWorkQueueStats]);

  useEffect(() => {
    if (!runId) return;

    const interval = setInterval(async () => {
      try {
        const requests = [fetchWorkQueueStats()];
        const sseUnsupported = typeof EventSource === "undefined";

        if (!isConnected || sseUnsupported) {
          requests.push(fetchLatestAggregated(), fetchRun());
        }

        await Promise.all(requests);
        setError(null);
        setIsConnected(true);
        setLastUpdate(formatTime());
      } catch (err) {
        setError(err);
        setIsConnected(false);
      }
    }, pollIntervalMs);

    return () => clearInterval(interval);
  }, [runId, pollIntervalMs, isConnected, fetchLatestAggregated, fetchWorkQueueStats, fetchRun]);

  useEffect(() => {
    if (!runId) return;
    if (typeof EventSource === "undefined") return;

    const source = createRunStatsEventSource(runId, streamIntervalMs);

    source.onopen = () => setIsConnected(true);

    source.addEventListener("stats", (event) => {
      try {
        const payload = JSON.parse(event.data);
        if (payload.run) setRun(payload.run);
        if (payload.aggregated) {
          setLatestAggregated(payload.aggregated);
          mergeLatest(payload.aggregated);
        }
        setError(null);
        setLastUpdate(formatTime());
        setIsConnected(true);
      } catch (err) {
        setError(err);
      }
    });

    source.addEventListener("error", (event) => {
      try {
        const payload = JSON.parse(event.data);
        setError(new Error(payload.error || "Server stream error"));
      } catch {
        setError(new Error("Server stream error"));
      }
      setIsConnected(false);
    });

    source.onerror = () => {
      setIsConnected(false);
    };

    return () => {
      source.close();
    };
  }, [runId, streamIntervalMs, mergeLatest]);

  const value = useMemo(
    () => ({
      runId,
      run,
      workQueueStats,
      history,
      latestAggregated,
      isConnected,
      lastUpdate,
      error,
      refreshHistory: fetchAggregatedHistory,
      refreshLatest: fetchLatestAggregated,
    }),
    [
      runId,
      run,
      workQueueStats,
      history,
      latestAggregated,
      isConnected,
      lastUpdate,
      error,
      fetchAggregatedHistory,
      fetchLatestAggregated,
    ],
  );

  return <RunHistoryContext.Provider value={value}>{children}</RunHistoryContext.Provider>;
};

export const useRunHistory = () => {
  const ctx = useContext(RunHistoryContext);
  if (!ctx) {
    throw new Error("useRunHistory must be used within RunHistoryProvider");
  }
  return ctx;
};
