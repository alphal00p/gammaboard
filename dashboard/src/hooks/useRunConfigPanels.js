import { useCallback } from "react";
import { fetchRunEvaluatorConfigPanels, fetchRunSamplerConfigPanels } from "../services/api";
import { usePanelSource } from "./usePanelSource";

export const useRunConfigPanels = ({ runId, pollMs = 5000 } = {}) => {
  const enabled = runId != null;

  const fetchEvaluatorPanels = useCallback(
    (_request, signal) => {
      if (!enabled) return null;
      return fetchRunEvaluatorConfigPanels(runId, signal);
    },
    [enabled, runId],
  );

  const fetchSamplerPanels = useCallback(
    (_request, signal) => {
      if (!enabled) return null;
      return fetchRunSamplerConfigPanels(runId, signal);
    },
    [enabled, runId],
  );

  const evaluator = usePanelSource({
    enabled,
    pollMs,
    fetchPanels: fetchEvaluatorPanels,
    useCursor: false,
  });
  const sampler = usePanelSource({
    enabled,
    pollMs,
    fetchPanels: fetchSamplerPanels,
    useCursor: false,
  });

  return {
    evaluator,
    sampler,
  };
};
