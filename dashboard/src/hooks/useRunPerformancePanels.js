import { useCallback } from "react";
import {
  fetchEvaluatorPerformanceHistory,
  fetchNodeEvaluatorPerformanceHistory,
  fetchSamplerPerformanceHistory,
} from "../services/api";
import { usePanelSource } from "./usePanelSource";

export const useRunPerformancePanels = ({ runId, evaluatorNodeName = null, limit = 200, pollMs = 5000 } = {}) => {
  const samplerEnabled = runId != null;
  const runEvaluatorEnabled = runId != null;
  const evaluatorEnabled = evaluatorNodeName != null;

  const fetchSamplerPanels = useCallback(
    (_request, signal) => {
      if (!samplerEnabled) return null;
      return fetchSamplerPerformanceHistory(runId, limit, null, signal);
    },
    [limit, runId, samplerEnabled],
  );

  const fetchEvaluatorPanels = useCallback(
    (_request, signal) => {
      if (!evaluatorEnabled) return null;
      return fetchNodeEvaluatorPerformanceHistory(evaluatorNodeName, limit, signal);
    },
    [evaluatorEnabled, evaluatorNodeName, limit],
  );
  const fetchRunEvaluatorPanels = useCallback(
    (_request, signal) => {
      if (!runEvaluatorEnabled) return null;
      return fetchEvaluatorPerformanceHistory(runId, limit, null, signal);
    },
    [limit, runEvaluatorEnabled, runId],
  );

  const sampler = usePanelSource({
    enabled: samplerEnabled,
    pollMs,
    fetchPanels: fetchSamplerPanels,
    useCursor: false,
  });
  const evaluator = usePanelSource({
    enabled: evaluatorEnabled,
    pollMs,
    fetchPanels: fetchEvaluatorPanels,
    useCursor: false,
  });
  const runEvaluator = usePanelSource({
    enabled: runEvaluatorEnabled,
    pollMs,
    fetchPanels: fetchRunEvaluatorPanels,
    useCursor: false,
  });

  return {
    evaluator,
    runEvaluator,
    sampler,
  };
};
