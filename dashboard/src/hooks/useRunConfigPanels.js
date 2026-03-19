import { useCallback, useMemo, useState } from "react";
import { fetchRunEvaluatorConfigPanels, fetchRunSamplerConfigPanels } from "../services/api";
import { usePolling } from "./usePolling";

const emptyResponse = Object.freeze({
  evaluator: null,
  sampler: null,
});

export const useRunConfigPanels = ({ runId, pollMs = 5000 } = {}) => {
  const [state, setState] = useState(emptyResponse);
  const enabled = runId != null;

  const poll = useCallback(
    async (signal) => {
      if (runId == null) return;
      try {
        const [evaluator, sampler] = await Promise.all([
          fetchRunEvaluatorConfigPanels(runId, signal),
          fetchRunSamplerConfigPanels(runId, signal),
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
    [runId],
  );

  const reset = useCallback(() => {
    setState(emptyResponse);
  }, []);

  usePolling({ enabled, intervalMs: pollMs, poll, reset });

  return useMemo(() => state, [state]);
};
