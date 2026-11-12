import { useCallback } from "react";
import { fetchCampaign } from "../services/api";
import { usePolledResource } from "./usePolledResource";

export function useCampaign({ runId, taskId, section = "settings", group = null, selection = null, paused = false }) {
  const parameters = JSON.stringify({ section, group, selection });
  const fetchResource = useCallback((signal) => fetchCampaign(runId, taskId, JSON.parse(parameters), signal), [runId, taskId, parameters]);
  return usePolledResource({ enabled: runId != null && taskId != null, pollMs: 10000, initialData: null, fetchResource, paused,
    cacheKey: section === "settings" ? null : `campaign:${runId}:${taskId}:${parameters}` });
}
