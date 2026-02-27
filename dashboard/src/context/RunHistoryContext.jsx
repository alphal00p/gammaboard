import React, { createContext, useCallback, useContext, useMemo, useEffect, useState } from "react";
import {
  createRunStatsEventSource,
  fetchAggregatedHistory as fetchAggregatedHistoryApi,
  fetchLatestAggregated as fetchLatestAggregatedApi,
  fetchRunLogs as fetchRunLogsApi,
  fetchRun as fetchRunApi,
  fetchStats as fetchStatsApi,
} from "../services/api";

const RunHistoryContext = createContext(null);

const formatTime = () => new Date().toLocaleTimeString();

const sameLogIdSet = (a, b) => {
  if (!Array.isArray(a) || !Array.isArray(b)) return false;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i += 1) {
    if (a[i]?.id !== b[i]?.id) return false;
  }
  return true;
};

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
  const [workerLogs, setWorkerLogs] = useState([]);
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

  const fetchAggregatedHistory = useCallback(
    async (signal) => {
      if (!runId) return;
      const data = await fetchAggregatedHistoryApi(runId, historyLimit, signal);
      setHistory(data);
      setLatestAggregated(data[0] || null);
    },
    [runId, historyLimit],
  );

  const fetchLatestAggregated = useCallback(
    async (signal) => {
      if (!runId) return;
      const data = await fetchLatestAggregatedApi(runId, signal);
      if (!data) {
        setLatestAggregated(null);
        return;
      }
      setLatestAggregated(data);
      mergeLatest(data);
    },
    [runId, mergeLatest],
  );

  const fetchRun = useCallback(
    async (signal) => {
      if (!runId) return;
      const data = await fetchRunApi(runId, signal);
      setRun(data);
    },
    [runId],
  );

  const fetchWorkQueueStats = useCallback(
    async (signal) => {
      if (!runId) return;
      const data = await fetchStatsApi(runId, signal);
      setWorkQueueStats(data);
    },
    [runId],
  );

  const fetchWorkerLogs = useCallback(
    async (signal) => {
      if (!runId) return;
      const data = await fetchRunLogsApi(runId, 500, null, null, signal);
      setWorkerLogs((prev) => (sameLogIdSet(prev, data) ? prev : data));
    },
    [runId],
  );

  useEffect(() => {
    if (!runId) {
      setHistory([]);
      setLatestAggregated(null);
      setRun(null);
      setWorkerLogs([]);
      setWorkQueueStats([]);
      setIsConnected(false);
      setLastUpdate(null);
      setError(null);
      return;
    }

    let cancelled = false;
    const controller = new AbortController();

    const loadInitial = async () => {
      try {
        await Promise.all([
          fetchAggregatedHistory(controller.signal),
          fetchRun(controller.signal),
          fetchWorkQueueStats(controller.signal),
          fetchWorkerLogs(controller.signal),
        ]);
        if (!cancelled) {
          setError(null);
          setIsConnected(true);
          setLastUpdate(formatTime());
        }
      } catch (err) {
        if (err?.name === "AbortError" || cancelled) return;
        if (!cancelled) {
          setError(err);
          setIsConnected(false);
        }
      }
    };

    loadInitial();

    return () => {
      cancelled = true;
      controller.abort();
    };
  }, [runId, fetchAggregatedHistory, fetchRun, fetchWorkQueueStats, fetchWorkerLogs]);

  useEffect(() => {
    if (!runId) return;
    let cancelled = false;
    let timeoutId;
    let activeController = null;

    const poll = async () => {
      activeController = new AbortController();
      try {
        const requests = [fetchWorkQueueStats(activeController.signal), fetchWorkerLogs(activeController.signal)];
        await Promise.all(requests);
        if (cancelled) return;
        setError(null);
        setIsConnected(true);
        setLastUpdate(formatTime());
      } catch (err) {
        if (err?.name === "AbortError" || cancelled) return;
        setError(err);
        setIsConnected(false);
      } finally {
        activeController = null;
        if (!cancelled) {
          timeoutId = setTimeout(poll, pollIntervalMs);
        }
      }
    };

    poll();

    return () => {
      cancelled = true;
      if (timeoutId) clearTimeout(timeoutId);
      if (activeController) activeController.abort();
    };
  }, [runId, pollIntervalMs, fetchWorkQueueStats, fetchWorkerLogs]);

  useEffect(() => {
    if (!runId) return;
    if (typeof EventSource !== "undefined") return;
    let cancelled = false;
    let timeoutId;
    let activeController = null;

    const pollStatsFallback = async () => {
      activeController = new AbortController();
      try {
        await Promise.all([fetchLatestAggregated(activeController.signal), fetchRun(activeController.signal)]);
        if (cancelled) return;
        setError(null);
        setIsConnected(true);
        setLastUpdate(formatTime());
      } catch (err) {
        if (err?.name === "AbortError" || cancelled) return;
        setError(err);
        setIsConnected(false);
      } finally {
        activeController = null;
        if (!cancelled) {
          timeoutId = setTimeout(pollStatsFallback, pollIntervalMs);
        }
      }
    };

    pollStatsFallback();

    return () => {
      cancelled = true;
      if (timeoutId) clearTimeout(timeoutId);
      if (activeController) activeController.abort();
    };
  }, [runId, pollIntervalMs, fetchLatestAggregated, fetchRun]);

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
      workerLogs,
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
      workerLogs,
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
