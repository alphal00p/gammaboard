import { useCallback, useMemo, useState } from "react";
import { fetchEvaluatorPerformanceHistory, fetchSamplerPerformanceHistory } from "../services/api";
import { usePolling } from "./usePolling";

const emptyResponse = Object.freeze({
  evaluator: null,
  sampler: null,
});

export const useRunPerformancePanels = ({ runId, limit = 200, pollMs = 5000 } = {}) => {
  const [state, setState] = useState(emptyResponse);
  const enabled = runId != null;

  const poll = useCallback(
    async (signal) => {
      if (runId == null) return;
      try {
        const [evaluator, sampler] = await Promise.all([
          fetchEvaluatorPerformanceHistory(runId, limit, null, signal),
          fetchSamplerPerformanceHistory(runId, limit, null, signal),
        ]);
        setState({
          evaluator: evaluator ?? null,
          sampler: sampler ?? null,
        });
      } catch (err) {
        if (err?.name === "AbortError") return;
        setState(emptyResponse);
      }
    },
    [limit, runId],
  );

  const reset = useCallback(() => {
    setState(emptyResponse);
  }, []);

  usePolling({ enabled, intervalMs: pollMs, poll, reset });

  return useMemo(() => state, [state]);
};
