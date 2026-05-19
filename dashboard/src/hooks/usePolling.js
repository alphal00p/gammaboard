import { useCallback, useEffect, useRef } from "react";

export const usePolling = ({ enabled = true, intervalMs, poll, reset = null }) => {
  const timeoutRef = useRef(null);
  const controllerRef = useRef(null);
  const cancelledRef = useRef(true);
  const runTokenRef = useRef(0);

  const clearScheduled = useCallback(() => {
    if (timeoutRef.current) {
      clearTimeout(timeoutRef.current);
      timeoutRef.current = null;
    }
  }, []);

  const run = useCallback(
    async (token) => {
      controllerRef.current = new AbortController();
      let nextIntervalMs = intervalMs;
      try {
        const requestedInterval = await poll(controllerRef.current.signal);
        if (requestedInterval === null || requestedInterval === false) {
          nextIntervalMs = null;
        } else if (Number.isFinite(Number(requestedInterval)) && Number(requestedInterval) > 0) {
          nextIntervalMs = Number(requestedInterval);
        }
      } finally {
        controllerRef.current = null;
        if (!cancelledRef.current && token === runTokenRef.current && nextIntervalMs != null) {
          timeoutRef.current = setTimeout(() => {
            const nextToken = runTokenRef.current + 1;
            runTokenRef.current = nextToken;
            run(nextToken);
          }, nextIntervalMs);
        }
      }
    },
    [intervalMs, poll],
  );

  const trigger = useCallback(() => {
    if (!enabled || cancelledRef.current) return;
    clearScheduled();
    runTokenRef.current += 1;
    if (controllerRef.current) controllerRef.current.abort();
    run(runTokenRef.current);
  }, [clearScheduled, enabled, run]);

  useEffect(() => {
    if (typeof reset === "function") reset();
    cancelledRef.current = !enabled;
    clearScheduled();
    if (controllerRef.current) controllerRef.current.abort();
    if (!enabled) return undefined;

    runTokenRef.current += 1;
    run(runTokenRef.current);

    return () => {
      cancelledRef.current = true;
      clearScheduled();
      if (controllerRef.current) controllerRef.current.abort();
    };
  }, [clearScheduled, enabled, run, reset]);

  return trigger;
};
