import { useCallback } from "react";
import { fetchEvaluatorPerformanceHistory, fetchSamplerPerformanceHistory } from "../services/api";
import { usePanelSource } from "./usePanelSource";

const usePerformanceSource = ({ enabled, pollMs, runId, limit, nodeName, fetchHistory }) => {
  const fetchPanels = useCallback(
    (_request, signal) => fetchHistory(runId, limit, nodeName, signal),
    [fetchHistory, limit, nodeName, runId],
  );
  return usePanelSource({ enabled, pollMs, fetchPanels, useCursor: false });
};

export const useRunPerformancePanels = ({ runId, evaluatorNodeName = null, limit = 200, pollMs = 5000 } = {}) => {
  const runEnabled = runId != null;
  const evaluatorEnabled = evaluatorNodeName != null;
  const sampler = usePerformanceSource({
    enabled: runEnabled,
    pollMs,
    runId,
    limit,
    nodeName: null,
    fetchHistory: fetchSamplerPerformanceHistory,
  });
  const evaluator = usePerformanceSource({
    enabled: evaluatorEnabled,
    pollMs,
    runId,
    limit,
    nodeName: evaluatorNodeName,
    fetchHistory: fetchEvaluatorPerformanceHistory,
  });
  const runEvaluator = usePerformanceSource({
    enabled: runEnabled,
    pollMs,
    runId,
    limit,
    nodeName: null,
    fetchHistory: fetchEvaluatorPerformanceHistory,
  });
  return { evaluator, runEvaluator, sampler };
};
