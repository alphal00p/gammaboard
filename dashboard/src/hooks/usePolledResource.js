import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { usePolling } from "./usePolling";

const formatTime = () => new Date().toLocaleTimeString(undefined, { hourCycle: "h23", hour: "2-digit", minute: "2-digit", second: "2-digit" });

export const usePolledResource = ({ enabled = true, paused = false, pollMs, initialData, fetchResource, onError = null } = {}) => {
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
        setLastUpdate(formatTime());
      } catch (err) {
        if (signal.aborted || err?.name === "AbortError") return;
        setError(err);
        setIsConnected(false);
        onErrorRef.current?.(err);
      } finally {
        if (!signal.aborted) setIsFetching(false);
      }
    },
    [enabled, fetchResource],
  );

  const reset = useCallback(() => {
    setData(initialDataRef.current); setError(null); setLastUpdate(null); setIsConnected(false); setIsFetching(false);
  }, []);
  const refresh = usePolling({ enabled, paused, intervalMs: pollMs, poll, reset });

  return useMemo(
    () => ({
      data: source === fetchResource ? data : initialDataRef.current,
      isConnected: source === fetchResource && isConnected,
      lastUpdate: source === fetchResource ? lastUpdate : null,
      error: source === fetchResource ? error : null,
      isLoading: enabled && (source !== fetchResource || (!lastUpdate && !error)),
      isFetching: enabled && (source !== fetchResource || isFetching),
      refresh,
    }),
    [data, error, isConnected, lastUpdate, source, fetchResource, enabled, isFetching, refresh],
  );
};
