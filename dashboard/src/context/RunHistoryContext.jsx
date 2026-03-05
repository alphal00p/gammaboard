import React, { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import {
  fetchAggregatedRange as fetchAggregatedRangeApi,
  fetchRun as fetchRunApi,
  fetchRunLogs as fetchRunLogsApi,
  fetchStats as fetchStatsApi,
} from "../services/api";

const RunMetaContext = createContext(null);
const RunConnectionContext = createContext(null);
const RunHeartbeatContext = createContext(null);
const RunAggregatedContext = createContext(null);
const RunQueueLogsContext = createContext(null);

const formatTime = () => new Date().toLocaleTimeString();

const mergeLogsAsc = (previous, incoming, maxSize) => {
  if (!Array.isArray(incoming) || incoming.length === 0) return previous;
  const out = Array.isArray(previous) ? [...previous] : [];
  const seen = new Set(out.map((entry) => entry?.id));
  let hasNew = false;
  for (const entry of incoming) {
    if (!entry || entry.id == null) continue;
    if (seen.has(entry.id)) continue;
    seen.add(entry.id);
    out.push(entry);
    hasNew = true;
  }
  if (!hasNew) return previous;
  return out.length > maxSize ? out.slice(out.length - maxSize) : out;
};

const normalizeRange = (start, stop) => (start <= stop ? [start, stop] : [stop, start]);

const computeStepForBuffer = (start, stop, bufferSize) => {
  const span = Math.max(1, stop - start + 1);
  const n = Math.max(1, Number(bufferSize) || 1);
  return Math.max(1, Math.ceil(span / n));
};

const dedupeNewestFirst = (entries, maxSize) => {
  if (!Array.isArray(entries) || entries.length === 0) return [];
  const seen = new Set();
  const out = [];
  for (const entry of entries) {
    const id = entry?.id;
    if (id == null || seen.has(id)) continue;
    seen.add(id);
    out.push(entry);
    if (out.length >= maxSize) break;
  }
  return out;
};

const toAscWithOptionalLatest = (payload) => {
  const snapshots = Array.isArray(payload?.snapshots) ? payload.snapshots : [];
  const latest = payload?.latest ?? null;
  if (!latest) return snapshots;
  const last = snapshots[snapshots.length - 1];
  if (last?.id === latest.id) return snapshots;
  return [...snapshots, latest];
};

const maxIdOf = (entries) => {
  let maxId = null;
  const bigIntCtor = typeof window !== "undefined" ? window.BigInt : undefined;
  if (typeof bigIntCtor !== "function") return null;
  for (const entry of entries || []) {
    if (entry?.id == null) continue;
    let id = null;
    try {
      id = bigIntCtor(entry.id);
    } catch {
      id = null;
    }
    if (id == null) continue;
    if (maxId == null || id > maxId) maxId = id;
  }
  return maxId == null ? null : maxId.toString();
};

export const RunHistoryProvider = ({
  runId,
  children,
  historyStart = -1000,
  historyStop = -1,
  historyBufferMax = 100,
  workerLogsLimit = 200,
  workQueueStatsLimit = 200,
  pollIntervalMs = 5000,
}) => {
  const [history, setHistory] = useState([]);
  const [latestAggregated, setLatestAggregated] = useState(null);
  const [run, setRun] = useState(null);
  const [workerLogs, setWorkerLogs] = useState([]);
  const [workQueueStats, setWorkQueueStats] = useState([]);
  const [isConnected, setIsConnected] = useState(false);
  const [lastUpdate, setLastUpdate] = useState(null);
  const [error, setError] = useState(null);
  const [isDocumentVisible, setIsDocumentVisible] = useState(() => {
    if (typeof document === "undefined") return true;
    return document.visibilityState === "visible";
  });

  const lastRangeRef = useRef(null);
  const latestSeenIdRef = useRef(null);
  const lastLogIdRef = useRef(null);

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
      const afterId = lastLogIdRef.current;
      const data = await fetchRunLogsApi(runId, workerLogsLimit, null, null, signal, afterId);
      const incoming = Array.isArray(data) ? data : [];
      if (afterId == null) {
        const trimmed = incoming.slice(-workerLogsLimit);
        setWorkerLogs(trimmed);
        const last = trimmed[trimmed.length - 1];
        if (last?.id != null) lastLogIdRef.current = last.id;
        return;
      }

      if (incoming.length === 0) return;
      setWorkerLogs((prev) => {
        const next = mergeLogsAsc(prev, incoming, workerLogsLimit);
        const last = next[next.length - 1];
        if (last?.id != null) lastLogIdRef.current = last.id;
        return next;
      });
    },
    [runId, workerLogsLimit],
  );

  const fetchAggregatedRange = useCallback(
    async (signal) => {
      if (!runId) return;

      const [rangeStart, rangeStop] = normalizeRange(historyStart, historyStop);
      const step = computeStepForBuffer(rangeStart, rangeStop, historyBufferMax);
      const currentRange = { start: rangeStart, stop: rangeStop, step };
      const rangeChanged =
        !lastRangeRef.current ||
        lastRangeRef.current.start !== currentRange.start ||
        lastRangeRef.current.stop !== currentRange.stop ||
        lastRangeRef.current.step !== currentRange.step;

      const latestId = rangeChanged ? null : latestSeenIdRef.current;
      const payload = await fetchAggregatedRangeApi(runId, rangeStart, rangeStop, step, latestId, signal);
      const ascEntries = toAscWithOptionalLatest(payload);
      const newestFirstIncoming = ascEntries.slice().reverse();

      if (rangeChanged || latestId == null) {
        const replaced = dedupeNewestFirst(newestFirstIncoming, historyBufferMax);
        setHistory(replaced);
      } else if (newestFirstIncoming.length > 0) {
        setHistory((prev) => dedupeNewestFirst([...newestFirstIncoming, ...prev], historyBufferMax));
      }

      const effectiveLatest = payload?.latest ?? ascEntries[ascEntries.length - 1] ?? null;
      if (effectiveLatest) setLatestAggregated(effectiveLatest);
      latestSeenIdRef.current = maxIdOf(ascEntries) ?? payload?.meta?.latest_id ?? latestSeenIdRef.current;
      lastRangeRef.current = currentRange;
    },
    [runId, historyStart, historyStop, historyBufferMax],
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
      latestSeenIdRef.current = null;
      lastRangeRef.current = null;
      lastLogIdRef.current = null;
      return;
    }

    let cancelled = false;
    let timeoutId;
    let activeController = null;

    const poll = async () => {
      activeController = new AbortController();
      try {
        const jobs = [
          fetchAggregatedRange(activeController.signal),
          fetchRun(activeController.signal),
          fetchWorkQueueStats(activeController.signal),
        ];
        if (isDocumentVisible || lastLogIdRef.current == null) {
          jobs.push(fetchWorkerLogs(activeController.signal));
        }
        await Promise.all(jobs);
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
  }, [runId, pollIntervalMs, fetchAggregatedRange, fetchRun, fetchWorkQueueStats, fetchWorkerLogs, isDocumentVisible]);

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
      refreshHistory: fetchAggregatedRange,
      refreshLatest: fetchAggregatedRange,
    }),
    [history, latestAggregated, fetchAggregatedRange],
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
