import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { readResourceCache, writeResourceCache, deleteResourceCache } from "../services/resourceCache";
import { usePolling } from "./usePolling";

const formatTime = () => new Date().toLocaleTimeString(undefined, { hourCycle: "h23", hour: "2-digit", minute: "2-digit", second: "2-digit" });

export const usePolledResource = ({ enabled = true, pollMs, initialData, fetchResource, onError = null, paused = false, cacheKey = null } = {}) => {
  const cached = useMemo(() => cacheKey ? readResourceCache(cacheKey) : null, [cacheKey]);
  const [data, setData] = useState(initialData);
  const [isConnected, setIsConnected] = useState(false);
  const [lastUpdate, setLastUpdate] = useState(null);
  const [error, setError] = useState(null);
  const [isFetching, setIsFetching] = useState(false);
  const [source, setSource] = useState(null);
  const initialDataRef = useRef(initialData);
  const onErrorRef = useRef(onError);

  useEffect(() => {
    initialDataRef.current = initialData;
  }, [initialData]);

  useEffect(() => {
    onErrorRef.current = onError;
  }, [onError]);

  const poll = useCallback(
    async (signal) => {
      if (!enabled || typeof fetchResource !== "function") {
        setData(initialDataRef.current);
        setError(null);
        setIsConnected(false);
        return;
      }

      setSource(() => fetchResource);
      setIsFetching(true);
      try {
        const nextData = await fetchResource(signal);
        if (signal.aborted) return;
        setData(nextData);
        setError(null);
        setIsConnected(true);
        const updated = formatTime();
        setLastUpdate(updated);
        if (cacheKey) writeResourceCache(cacheKey, { data: nextData, lastUpdate: updated });
      } catch (err) {
        if (signal.aborted || err?.name === "AbortError") return;
        if ([401,403,404].includes(err?.status)) { deleteResourceCache(cacheKey); setData(initialDataRef.current); }
        setError(err);
        setIsConnected(false);
        onErrorRef.current?.(err);
      } finally {
        if (!signal.aborted) setIsFetching(false);
      }
    },
    [enabled, fetchResource, cacheKey],
  );

  const reset = useCallback(() => {
    const saved = cacheKey ? readResourceCache(cacheKey) : null;
    setData(saved?.data ?? initialDataRef.current); setError(null); setLastUpdate(saved?.lastUpdate ?? null); setIsConnected(false); setIsFetching(false);
  }, [cacheKey]);
  const refresh = usePolling({ enabled, intervalMs: pollMs, poll, reset, paused });

  return useMemo(
    () => ({
      data: source === fetchResource ? data : cached?.data ?? initialDataRef.current,
      isConnected: source === fetchResource && isConnected,
      lastUpdate: source === fetchResource ? lastUpdate : cached?.lastUpdate ?? null,
      error: source === fetchResource ? error : null,
      isLoading: enabled && (source !== fetchResource ? !cached : !lastUpdate && !error),
      isFetching: enabled && (source !== fetchResource || isFetching),
      refresh,
    }),
    [data, error, isConnected, lastUpdate, source, fetchResource, enabled, isFetching, refresh, cached],
  );
};
