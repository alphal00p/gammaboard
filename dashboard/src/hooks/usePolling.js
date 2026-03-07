import { useEffect } from "react";

export const usePolling = ({ enabled = true, intervalMs, poll, reset = null }) => {
  useEffect(() => {
    if (typeof reset === "function") reset();
    if (!enabled) return undefined;

    let cancelled = false;
    let timeoutId;
    let activeController = null;

    const run = async () => {
      activeController = new AbortController();
      try {
        await poll(activeController.signal);
      } finally {
        activeController = null;
        if (!cancelled) timeoutId = setTimeout(run, intervalMs);
      }
    };

    run();

    return () => {
      cancelled = true;
      if (timeoutId) clearTimeout(timeoutId);
      if (activeController) activeController.abort();
    };
  }, [enabled, intervalMs, poll, reset]);
};
