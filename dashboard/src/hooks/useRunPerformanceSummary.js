import { useCallback, useMemo, useState } from "react";
import { fetchEvaluatorPerformanceHistory, fetchSamplerPerformanceHistory } from "../services/api";
import { usePolling } from "./usePolling";

const emptyState = Object.freeze({
  evaluatorEntries: [],
  samplerEntries: [],
  latestEvaluator: null,
  latestSampler: null,
});

export const useRunPerformanceSummary = ({ runId, limit = 200, pollMs = 5000 } = {}) => {
  const [state, setState] = useState(emptyState);

  const enabled = runId != null;
  const poll = useCallback(
    async (signal) => {
      if (runId == null) return;
      try {
        const [evaluatorRows, samplerRows] = await Promise.all([
          fetchEvaluatorPerformanceHistory(runId, limit, null, signal),
          fetchSamplerPerformanceHistory(runId, limit, null, signal),
        ]);
        const normalizedEvaluatorRows = Array.isArray(evaluatorRows) ? evaluatorRows : [];
        const normalizedSamplerRows = Array.isArray(samplerRows) ? samplerRows : [];
        setState({
          evaluatorEntries: normalizedEvaluatorRows,
          samplerEntries: normalizedSamplerRows,
          latestEvaluator: normalizedEvaluatorRows.length > 0 ? normalizedEvaluatorRows[0] : null,
          latestSampler: normalizedSamplerRows.length > 0 ? normalizedSamplerRows[0] : null,
        });
      } catch (err) {
        if (err?.name === "AbortError") return;
        setState(emptyState);
      }
    },
    [runId, limit],
  );

  const reset = useCallback(() => {
    setState(emptyState);
  }, []);

  usePolling({ enabled, intervalMs: pollMs, poll, reset });

  return useMemo(() => state, [state]);
};
