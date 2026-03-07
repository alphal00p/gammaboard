import React, { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import {
  fetchAggregatedRange as fetchAggregatedRangeApi,
  fetchRun as fetchRunApi,
  fetchStats as fetchStatsApi,
} from "../services/api";

const RunHistoryContext = createContext(null);

const formatTime = () => new Date().toLocaleTimeString();
const isConnectivityError = (err) => !(err && err.isHttp === true);

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
  workQueueStatsLimit = 200,
  pollIntervalMs = 5000,
}) => {
  const [history, setHistory] = useState([]);
  const [latestAggregated, setLatestAggregated] = useState(null);
  const [run, setRun] = useState(null);
  const [workQueueStats, setWorkQueueStats] = useState([]);
  const [isConnected, setIsConnected] = useState(false);
  const [lastUpdate, setLastUpdate] = useState(null);
  const [error, setError] = useState(null);

  const lastRangeRef = useRef(null);
  const latestSeenIdRef = useRef(null);

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

  const fetchAggregatedRange = useCallback(
    async (signal) => {
      if (!runId) return;

      const currentRange = { start: historyStart, stop: historyStop };
      const previousRange = lastRangeRef.current;
      const rangeChanged =
        !previousRange || previousRange.start !== currentRange.start || previousRange.stop !== currentRange.stop;

      const latestId = rangeChanged ? null : latestSeenIdRef.current;
      const payload = await fetchAggregatedRangeApi(
        runId,
        currentRange.start,
        currentRange.stop,
        historyBufferMax,
        latestId,
        signal,
      );
      const ascEntries = toAscWithOptionalLatest(payload);
      // Backend guarantees ascending id order for sampled range snapshots.
      const newestFirstIncoming = ascEntries.slice().reverse();
      const returnedStep = Number(payload?.meta?.step) || 1;
      const shouldReset = rangeChanged || latestId == null || payload?.reset_required === true;

      if (shouldReset) {
        const replaced = dedupeNewestFirst(newestFirstIncoming, historyBufferMax);
        setHistory(replaced);
      } else if (newestFirstIncoming.length > 0) {
        setHistory((prev) => dedupeNewestFirst([...newestFirstIncoming, ...prev], historyBufferMax));
      }

      const effectiveLatest = payload?.latest ?? ascEntries[ascEntries.length - 1] ?? null;
      if (effectiveLatest) setLatestAggregated(effectiveLatest);
      latestSeenIdRef.current = maxIdOf(ascEntries) ?? payload?.meta?.latest_id ?? latestSeenIdRef.current;
      lastRangeRef.current = { ...currentRange, step: returnedStep };
    },
    [runId, historyStart, historyStop, historyBufferMax],
  );

  useEffect(() => {
    // Always clear run-scoped state first so switching runs never shows stale data.
    setHistory([]);
    setLatestAggregated(null);
    setRun(null);
    setWorkQueueStats([]);
    setIsConnected(false);
    setLastUpdate(null);
    setError(null);
    latestSeenIdRef.current = null;
    lastRangeRef.current = null;

    if (!runId) return;

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
        const results = await Promise.allSettled(jobs);
        if (cancelled) return;

        const failures = results
          .filter((result) => result.status === "rejected")
          .map((result) => result.reason)
          .filter((err) => err?.name !== "AbortError");

        if (failures.length > 0) {
          setError(failures[0]);
          setIsConnected(!failures.some(isConnectivityError));
        } else {
          setError(null);
          setIsConnected(true);
        }
        setLastUpdate(formatTime());
      } catch (err) {
        if (err?.name === "AbortError" || cancelled) return;
        setError(err);
        setIsConnected(!isConnectivityError(err));
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
  }, [runId, pollIntervalMs, fetchAggregatedRange, fetchRun, fetchWorkQueueStats]);

  const value = useMemo(
    () => ({
      runId,
      run,
      isConnected,
      error,
      lastUpdate,
      history,
      latestAggregated,
      refresh: fetchAggregatedRange,
      workQueueStats,
      refreshWorkQueueStats: fetchWorkQueueStats,
    }),
    [
      runId,
      run,
      isConnected,
      error,
      lastUpdate,
      history,
      latestAggregated,
      fetchAggregatedRange,
      workQueueStats,
      fetchWorkQueueStats,
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
