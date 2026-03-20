import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { usePolling } from "./usePolling";

const formatTime = () => new Date().toLocaleTimeString();

export const usePolledResource = ({ enabled = true, pollMs, initialData, fetchResource, onError = null } = {}) => {
  const [data, setData] = useState(initialData);
  const [isConnected, setIsConnected] = useState(false);
  const [lastUpdate, setLastUpdate] = useState(null);
  const [error, setError] = useState(null);
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

      try {
        const nextData = await fetchResource(signal);
        setData(nextData);
        setError(null);
        setIsConnected(true);
        setLastUpdate(formatTime());
      } catch (err) {
        if (err?.name === "AbortError") return;
        setData(initialDataRef.current);
        setError(err);
        setIsConnected(false);
        onErrorRef.current?.(err);
      }
    },
    [enabled, fetchResource],
  );

  usePolling({ enabled, intervalMs: pollMs, poll });

  return useMemo(
    () => ({
      data,
      isConnected,
      lastUpdate,
      error,
    }),
    [data, error, isConnected, lastUpdate],
  );
};
