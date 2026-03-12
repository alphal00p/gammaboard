import { useCallback, useState } from "react";
import { fetchNodeEvaluatorPerformanceHistory, fetchNodeSamplerPerformanceHistory } from "../services/api";
import { usePolling } from "./usePolling";

const emptyResponse = Object.freeze({ run_id: null, entries: [] });

export const useWorkerPerformanceHistory = ({ workerId, role, limit = 200, pollMs = 5000 } = {}) => {
  const [data, setData] = useState(emptyResponse);

  const enabled = Boolean(workerId && (role === "evaluator" || role === "sampler_aggregator"));
  const poll = useCallback(
    async (signal) => {
      try {
        const payload =
          role === "evaluator"
            ? await fetchNodeEvaluatorPerformanceHistory(workerId, limit, signal)
            : await fetchNodeSamplerPerformanceHistory(workerId, limit, signal);
        setData(payload || emptyResponse);
      } catch (err) {
        if (err?.name === "AbortError") return;
        setData(emptyResponse);
      }
    },
    [workerId, role, limit],
  );
  const reset = useCallback(() => {
    setData(emptyResponse);
  }, []);

  usePolling({ enabled, intervalMs: pollMs, poll, reset });

  return data;
};
