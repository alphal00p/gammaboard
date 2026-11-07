import { useCallback, useEffect, useRef } from "react";
import { useWorkspaceActive } from "../components/common/WorkspaceTabs";

export const usePolling = ({ enabled = true, intervalMs, poll, reset = null, paused = false }) => {
  const workspaceActive = useWorkspaceActive();
  const active = workspaceActive && !paused;
  const resetSource = useRef(null);
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
      const controller = new AbortController();
      controllerRef.current = controller;
      let nextIntervalMs = intervalMs;
      try {
        const requestedInterval = await poll(controller.signal);
        if (requestedInterval === null || requestedInterval === false) {
          nextIntervalMs = null;
        } else if (Number.isFinite(Number(requestedInterval)) && Number(requestedInterval) > 0) {
          nextIntervalMs = Number(requestedInterval);
        }
      } finally {
        if (controllerRef.current === controller) controllerRef.current = null;
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
    if (!enabled || !active || cancelledRef.current) return;
    clearScheduled();
    runTokenRef.current += 1;
    if (controllerRef.current) controllerRef.current.abort();
    run(runTokenRef.current);
  }, [clearScheduled, enabled, active, run]);

  useEffect(() => {
    const previous = resetSource.current;
    if (!previous || previous.poll !== poll || previous.reset !== reset || previous.enabled !== enabled) {
      reset?.();
      resetSource.current = { poll, reset, enabled };
    }
    cancelledRef.current = !enabled || !active;
    clearScheduled();
    if (controllerRef.current) controllerRef.current.abort();
    if (!enabled || !active) return undefined;

    runTokenRef.current += 1;
    run(runTokenRef.current);

    return () => {
      cancelledRef.current = true;
      runTokenRef.current += 1;
      clearScheduled();
      if (controllerRef.current) controllerRef.current.abort();
    };
  }, [clearScheduled, enabled, active, run, reset, poll]);

  return trigger;
};
