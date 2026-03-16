import { useCallback, useState } from "react";
import { fetchRunTasks } from "../services/api";
import { usePolling } from "./usePolling";

export const useRunTasks = (runId, refreshInterval = 2000) => {
  const [tasks, setTasks] = useState([]);

  const poll = useCallback(
    async (signal) => {
      if (runId == null) {
        setTasks([]);
        return;
      }
      try {
        const data = await fetchRunTasks(runId, signal);
        setTasks(data);
      } catch (err) {
        if (err?.name === "AbortError") return;
        console.error("Failed to fetch run tasks:", err);
        setTasks([]);
      }
    },
    [runId],
  );

  usePolling({ intervalMs: refreshInterval, poll });

  return { tasks };
};
