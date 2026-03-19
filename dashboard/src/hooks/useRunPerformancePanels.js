import { useCallback } from "react";
import {
  fetchNodeEvaluatorPerformanceHistory,
  fetchSamplerPerformanceHistory,
} from "../services/api";
import { usePanelSource } from "./usePanelSource";

export const useRunPerformancePanels = ({
  runId,
  evaluatorNodeId = null,
  limit = 200,
  pollMs = 5000,
} = {}) => {
  const samplerEnabled = runId != null;
  const evaluatorEnabled = evaluatorNodeId != null;

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
      return fetchNodeEvaluatorPerformanceHistory(evaluatorNodeId, limit, signal);
    },
    [evaluatorEnabled, evaluatorNodeId, limit],
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

  return {
    evaluator,
    sampler,
  };
};
