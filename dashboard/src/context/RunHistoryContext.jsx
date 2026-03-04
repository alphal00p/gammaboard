import React, { createContext, useCallback, useContext, useMemo, useEffect, useState } from "react";
import {
  createRunStatsEventSource,
  fetchAggregatedHistory as fetchAggregatedHistoryApi,
  fetchLatestAggregated as fetchLatestAggregatedApi,
  fetchRunLogs as fetchRunLogsApi,
  fetchRun as fetchRunApi,
  fetchStats as fetchStatsApi,
} from "../services/api";

const RunMetaContext = createContext(null);
const RunConnectionContext = createContext(null);
const RunHeartbeatContext = createContext(null);
const RunAggregatedContext = createContext(null);
const RunQueueLogsContext = createContext(null);

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
  historyBufferMax = 500,
  workerLogsLimit = 200,
  workQueueStatsLimit = 200,
  pollIntervalMs = 5000,
  sseConnectedPollThrottleFactor = 4,
  streamIntervalMs = 1000,
}) => {
  const [history, setHistory] = useState([]);
  const [latestAggregated, setLatestAggregated] = useState(null);
  const [run, setRun] = useState(null);
  const [workerLogs, setWorkerLogs] = useState([]);
  const [workQueueStats, setWorkQueueStats] = useState([]);
  const [isConnected, setIsConnected] = useState(false);
  const [isSseConnected, setIsSseConnected] = useState(false);
  const [lastUpdate, setLastUpdate] = useState(null);
  const [error, setError] = useState(null);
  const [isDocumentVisible, setIsDocumentVisible] = useState(() => {
    if (typeof document === "undefined") return true;
    return document.visibilityState === "visible";
  });

  const trimHistory = useCallback(
    (entries) => {
      if (!Array.isArray(entries) || entries.length === 0) return [];
      const seen = new Set();
      const trimmed = [];
      for (const entry of entries) {
        const id = entry?.id;
        if (id == null || seen.has(id)) continue;
        seen.add(id);
        trimmed.push(entry);
        if (trimmed.length >= historyBufferMax) break;
      }
      return trimmed;
    },
    [historyBufferMax],
  );

  const mergeLatest = useCallback(
    (next) => {
      if (!next) return;
      setHistory((prev) => trimHistory([next, ...prev]));
    },
    [trimHistory],
  );

  const fetchAggregatedHistory = useCallback(
    async (signal) => {
      if (!runId) return;
      const data = await fetchAggregatedHistoryApi(runId, historyLimit, signal);
      const trimmed = trimHistory(data);
      setHistory(trimmed);
      setLatestAggregated(trimmed[0] || null);
    },
    [runId, historyLimit, trimHistory],
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
      setWorkQueueStats((Array.isArray(data) ? data : []).slice(0, workQueueStatsLimit));
    },
    [runId, workQueueStatsLimit],
  );

  const fetchWorkerLogs = useCallback(
    async (signal) => {
      if (!runId) return;
      const data = await fetchRunLogsApi(runId, workerLogsLimit, null, null, signal);
      const trimmed = (Array.isArray(data) ? data : []).slice(0, workerLogsLimit);
      setWorkerLogs((prev) => (sameLogIdSet(prev, trimmed) ? prev : trimmed));
    },
    [runId, workerLogsLimit],
  );

  useEffect(() => {
    if (typeof document === "undefined") return undefined;
    const handleVisibilityChange = () => {
      setIsDocumentVisible(document.visibilityState === "visible");
    };
    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, []);

  useEffect(() => {
    if (!runId) {
      setHistory([]);
      setLatestAggregated(null);
      setRun(null);
      setWorkerLogs([]);
      setWorkQueueStats([]);
      setIsConnected(false);
      setIsSseConnected(false);
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
          ...(isDocumentVisible ? [fetchWorkerLogs(controller.signal)] : []),
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
  }, [runId, fetchAggregatedHistory, fetchRun, fetchWorkQueueStats, fetchWorkerLogs, isDocumentVisible]);

  useEffect(() => {
    if (!runId) return;
    let cancelled = false;
    let timeoutId;
    let activeController = null;

    const poll = async () => {
      activeController = new AbortController();
      try {
        const requests = [fetchWorkQueueStats(activeController.signal)];
        if (isDocumentVisible) {
          requests.push(fetchWorkerLogs(activeController.signal));
        }
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
          const effectiveIntervalMs =
            typeof EventSource !== "undefined" && isSseConnected
              ? pollIntervalMs * sseConnectedPollThrottleFactor
              : pollIntervalMs;
          timeoutId = setTimeout(poll, effectiveIntervalMs);
        }
      }
    };

    poll();

    return () => {
      cancelled = true;
      if (timeoutId) clearTimeout(timeoutId);
      if (activeController) activeController.abort();
    };
  }, [
    runId,
    pollIntervalMs,
    fetchWorkQueueStats,
    fetchWorkerLogs,
    isDocumentVisible,
    isSseConnected,
    sseConnectedPollThrottleFactor,
  ]);

  useEffect(() => {
    if (!runId) return;
    if (typeof EventSource !== "undefined") return;
    setIsSseConnected(false);
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

    source.onopen = () => {
      setIsConnected(true);
      setIsSseConnected(true);
    };

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
        setIsSseConnected(true);
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
      setIsSseConnected(false);
    });

    source.onerror = () => {
      setIsConnected(false);
      setIsSseConnected(false);
    };

    return () => {
      source.close();
    };
  }, [runId, streamIntervalMs, mergeLatest]);

  const metaValue = useMemo(
    () => ({
      runId,
      run,
    }),
    [runId, run],
  );

  const connectionValue = useMemo(
    () => ({
      isConnected,
      error,
    }),
    [isConnected, error],
  );

  const heartbeatValue = useMemo(
    () => ({
      lastUpdate,
    }),
    [lastUpdate],
  );

  const aggregatedValue = useMemo(
    () => ({
      history,
      latestAggregated,
      refreshHistory: fetchAggregatedHistory,
      refreshLatest: fetchLatestAggregated,
    }),
    [history, latestAggregated, fetchAggregatedHistory, fetchLatestAggregated],
  );

  const queueLogsValue = useMemo(
    () => ({
      workerLogs,
      workQueueStats,
      refreshWorkQueueStats: fetchWorkQueueStats,
      refreshWorkerLogs: fetchWorkerLogs,
    }),
    [workerLogs, workQueueStats, fetchWorkQueueStats, fetchWorkerLogs],
  );

  return (
    <RunMetaContext.Provider value={metaValue}>
      <RunConnectionContext.Provider value={connectionValue}>
        <RunHeartbeatContext.Provider value={heartbeatValue}>
          <RunAggregatedContext.Provider value={aggregatedValue}>
            <RunQueueLogsContext.Provider value={queueLogsValue}>{children}</RunQueueLogsContext.Provider>
          </RunAggregatedContext.Provider>
        </RunHeartbeatContext.Provider>
      </RunConnectionContext.Provider>
    </RunMetaContext.Provider>
  );
};

export const useRunState = () => {
  const ctx = useContext(RunMetaContext);
  if (!ctx) {
    throw new Error("useRunState must be used within RunHistoryProvider");
  }
  return ctx;
};

export const useRunConnection = () => {
  const ctx = useContext(RunConnectionContext);
  if (!ctx) {
    throw new Error("useRunConnection must be used within RunHistoryProvider");
  }
  return ctx;
};

export const useRunHeartbeat = () => {
  const ctx = useContext(RunHeartbeatContext);
  if (!ctx) {
    throw new Error("useRunHeartbeat must be used within RunHistoryProvider");
  }
  return ctx;
};

export const useRunAggregated = () => {
  const ctx = useContext(RunAggregatedContext);
  if (!ctx) {
    throw new Error("useRunAggregated must be used within RunHistoryProvider");
  }
  return ctx;
};

export const useRunQueueLogs = () => {
  const ctx = useContext(RunQueueLogsContext);
  if (!ctx) {
    throw new Error("useRunQueueLogs must be used within RunHistoryProvider");
  }
  return ctx;
};

export const useRunHistory = () => {
  const meta = useRunState();
  const connection = useRunConnection();
  const heartbeat = useRunHeartbeat();
  const aggregated = useRunAggregated();
  const queueLogs = useRunQueueLogs();
  const ctx = useMemo(
    () => ({
      ...meta,
      ...connection,
      ...heartbeat,
      ...aggregated,
      ...queueLogs,
    }),
    [meta, connection, heartbeat, aggregated, queueLogs],
  );
  if (!ctx) {
    throw new Error("useRunHistory must be used within RunHistoryProvider");
  }
  return ctx;
};
