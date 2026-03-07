import { useCallback, useRef, useState } from "react";
import { fetchRunLogs } from "../services/api";
import { mergeLogsAsc } from "../utils/logs";
import { usePolling } from "./usePolling";

export const useWorkerLogs = ({ runId, workerId = null, limit = 200, pollMs = 5000 } = {}) => {
  const [logs, setLogs] = useState([]);
  const lastLogIdRef = useRef(null);

  const reset = useCallback(() => {
    setLogs([]);
    lastLogIdRef.current = null;
  }, []);

  const poll = useCallback(
    async (signal) => {
      try {
        const afterId = lastLogIdRef.current;
        const data = await fetchRunLogs(runId, limit, workerId, null, signal, afterId);
        const incoming = Array.isArray(data) ? data : [];
        if (afterId == null) {
          const trimmed = incoming.slice(-limit);
          setLogs(trimmed);
          const last = trimmed[trimmed.length - 1];
          if (last?.id != null) lastLogIdRef.current = last.id;
          return;
        }
        if (incoming.length === 0) return;
        setLogs((previous) => {
          const next = mergeLogsAsc(previous, incoming, limit);
          const last = next[next.length - 1];
          if (last?.id != null) lastLogIdRef.current = last.id;
          return next;
        });
      } catch (err) {
        if (err?.name === "AbortError") return;
      }
    },
    [runId, workerId, limit],
  );

  usePolling({ enabled: runId != null, intervalMs: pollMs, poll, reset });

  return logs;
};
