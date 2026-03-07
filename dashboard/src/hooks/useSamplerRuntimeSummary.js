import { useCallback, useState } from "react";
import { fetchWorkers } from "../services/api";
import { usePolling } from "./usePolling";

const rollingMean = (metric) => {
  if (Number.isFinite(Number(metric))) return Number(metric);
  if (!metric || typeof metric !== "object" || Array.isArray(metric)) return null;
  const mean = Number(metric.mean);
  return Number.isFinite(mean) ? mean : null;
};

export const useSamplerRuntimeSummary = (runId, pollMs = 3000) => {
  const [summary, setSummary] = useState(null);

  const poll = useCallback(
    async (signal) => {
      try {
        const workers = await fetchWorkers(runId, signal);
        const list = Array.isArray(workers) ? workers : [];
        const samplerWorker =
          list.find((worker) => worker.role === "sampler_aggregator" && worker.status === "active") ||
          list.find((worker) => worker.role === "sampler_aggregator") ||
          null;
        const rolling = samplerWorker?.sampler_runtime_metrics?.rolling || {};
        const remainingRatio = rollingMean(rolling.queue_remaining_ratio);
        const avgQueueDepletion = remainingRatio == null ? null : Math.max(0, Math.min(1, 1 - remainingRatio));
        setSummary({
          avg_queue_depletion: avgQueueDepletion,
          avg_time_per_sample_ms: rollingMean(rolling.eval_ms_per_sample),
          avg_time_per_batch_ms: rollingMean(rolling.eval_ms_per_batch),
        });
      } catch (err) {
        if (err?.name === "AbortError") return;
        setSummary(null);
      }
    },
    [runId],
  );
  const reset = useCallback(() => {
    setSummary(null);
  }, []);

  usePolling({ enabled: Boolean(runId), intervalMs: pollMs, poll, reset });

  return summary;
};
