import { useCallback, useMemo, useState } from "react";
import {
  fetchNodeEvaluatorPerformanceHistory,
  fetchSamplerPerformanceHistory,
} from "../services/api";
import { usePolling } from "./usePolling";

const emptyResponse = Object.freeze({
  evaluator: null,
  sampler: null,
});

export const useRunPerformancePanels = ({
  runId,
  evaluatorNodeId = null,
  limit = 200,
  pollMs = 5000,
} = {}) => {
  const [state, setState] = useState(emptyResponse);
  const enabled = runId != null;

  const poll = useCallback(
    async (signal) => {
      if (runId == null) return;
      try {
        const [sampler, evaluator] = await Promise.all([
          fetchSamplerPerformanceHistory(runId, limit, null, signal),
          evaluatorNodeId ? fetchNodeEvaluatorPerformanceHistory(evaluatorNodeId, limit, signal) : Promise.resolve(null),
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
    [evaluatorNodeId, limit, runId],
  );

  const reset = useCallback(() => {
    setState(emptyResponse);
  }, []);

  usePolling({ enabled, intervalMs: pollMs, poll, reset });

  return useMemo(() => state, [state]);
};
