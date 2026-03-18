import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { fetchRunTaskOutput, fetchRunTaskOutputHistory } from "../services/api";
import { usePolling } from "./usePolling";

const emptyState = Object.freeze({
  output: null,
  historyItems: [],
  latestSnapshotId: null,
});

const mergeHistoryItems = (previousItems, nextItems) => {
  const items = [...previousItems, ...(Array.isArray(nextItems) ? nextItems : [])];
  const seen = new Set();
  return items.filter((item) => {
    const id = item?.snapshot_id;
    if (!id || seen.has(id)) return false;
    seen.add(id);
    return true;
  });
};

export const useTaskOutput = ({ runId, taskId, pollMs = 3000, historyLimit = 500 } = {}) => {
  const [state, setState] = useState(emptyState);
  const latestSnapshotIdRef = useRef(null);
  const enabled = runId != null && taskId != null;

  useEffect(() => {
    latestSnapshotIdRef.current = state.latestSnapshotId;
  }, [state.latestSnapshotId]);

  const poll = useCallback(
    async (signal) => {
      if (!enabled) return;
      try {
        const currentSnapshotId = latestSnapshotIdRef.current;
        const [output, history] = await Promise.all([
          fetchRunTaskOutput(runId, taskId, signal),
          fetchRunTaskOutputHistory(
            runId,
            taskId,
            {
              limit: historyLimit,
              afterSnapshotId: currentSnapshotId,
            },
            signal,
          ),
        ]);

        setState((previous) => {
          const shouldReset = history?.reset_required === true || previous.output?.task_id !== output?.task_id;
          const historyItems = shouldReset
            ? Array.isArray(history?.items)
              ? history.items
              : []
            : mergeHistoryItems(previous.historyItems, history?.items);

          return {
            output: output ?? null,
            historyItems,
            latestSnapshotId: history?.latest_snapshot_id ?? output?.latest_snapshot_id ?? previous.latestSnapshotId,
          };
        });
      } catch (err) {
        if (err?.name === "AbortError") return;
        setState(emptyState);
      }
    },
    [enabled, historyLimit, runId, taskId],
  );

  const reset = useCallback(() => {
    latestSnapshotIdRef.current = null;
    setState(emptyState);
  }, []);

  usePolling({ enabled, intervalMs: pollMs, poll, reset });

  return useMemo(() => state, [state]);
};
