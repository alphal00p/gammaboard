const APP_BASE_URL = import.meta.env.BASE_URL || "/";
const normalizedAppBase = APP_BASE_URL.endsWith("/") ? APP_BASE_URL : `${APP_BASE_URL}/`;
const API_BASE_URL = `${normalizedAppBase}api`;
export const apiUrl = (path = "") => `${API_BASE_URL}${path.startsWith("/") ? path : `/${path}`}`;

const stripHtml = (value) =>
  value
    .replace(/<[^>]*>/g, " ")
    .replace(/\s+/g, " ")
    .trim();

const extractErrorDetails = async (response) => {
  const contentType = response.headers.get("content-type") || "";

  try {
    if (contentType.includes("application/json")) {
      const payload = await response.json();
      if (typeof payload?.error === "string" && payload.error.trim()) return payload.error.trim();
      if (typeof payload?.message === "string" && payload.message.trim()) return payload.message.trim();
      if (typeof payload === "string" && payload.trim()) return payload.trim();
      return JSON.stringify(payload);
    }

    const text = await response.text();
    if (text.trim()) {
      if (contentType.includes("text/html")) {
        const summary = stripHtml(text);
        if (response.status === 502 || response.status === 503 || response.status === 504) {
          return `gateway error (${response.status}): backend unavailable or restarting`;
        }
        return summary || `HTTP ${response.status}`;
      }
      return text.trim();
    }
  } catch {
    // Fall through to status fallback.
  }

  if (response.status === 502 || response.status === 503 || response.status === 504) {
    return `gateway error (${response.status}): backend unavailable or restarting`;
  }

  return response.statusText || `HTTP ${response.status}`;
};

const parseJsonOrThrow = async (response, message) => {
  if (!response.ok) {
    const details = await extractErrorDetails(response);
    const error = new Error(`${message}: ${details}`);
    error.status = response.status;
    error.isHttp = true;
    throw error;
  }
  return response.json();
};

const buildQueryString = (entries) => {
  const params = new URLSearchParams();
  for (const [key, value] of entries) {
    if (value == null) continue;
    const text = typeof value === "string" ? value.trim() : String(value);
    if (!text) continue;
    params.set(key, text);
  }
  const query = params.toString();
  return query ? `?${query}` : "";
};

const apiRequest = async (path, message, signal, options = {}) => {
  const response = await fetch(apiUrl(path), {
    credentials: "include",
    signal,
    ...options,
  });
  return parseJsonOrThrow(response, message);
};

const apiGet = async (path, message, signal) => apiRequest(path, message, signal);

const apiPost = async (path, payload, message, signal) =>
  apiRequest(path, message, signal, {
    method: "POST",
    headers: {
      "content-type": "application/json",
    },
    body: JSON.stringify(payload ?? {}),
  });

const apiDelete = async (path, message, signal) =>
  apiRequest(path, message, signal, {
    method: "DELETE",
  });

export const fetchRuns = async ({ includeChildren = false, limit = 100, offset = 0 } = {}, signal) => {
  const data = await apiGet(
    `/runs${buildQueryString([
      ["include_children", includeChildren ? "true" : null],
      ["limit", limit],
      ["offset", offset],
    ])}`,
    "Failed to fetch runs",
    signal,
  );
  return {
    items: data.items,
    nextOffset: data.next_offset,
  };
};

export const fetchServerStatus = async (signal) => apiGet("/health", "Failed to fetch server status", signal);

// POST makes browsers include Origin even through the same-origin dashboard
// proxy, so a forwarded port mismatch is detected before mounting workspaces.
export const fetchSession = async (signal) => apiPost("/auth/session", {}, "Failed to check browser access", signal);

export const fetchSettingsOverview = async (signal) => apiGet("/settings", "Failed to fetch settings", signal);

export const login = async (password, signal) => apiPost("/auth/login", { password }, "Failed to log in", signal);

export const logout = async (signal) => apiPost("/auth/logout", {}, "Failed to log out", signal);

export const pauseRun = async (runId, signal) => apiPost(`/runs/${runId}/pause`, {}, "Failed to pause run", signal);

export const createRun = async (toml, signal) => apiPost("/runs", { toml }, "Failed to create run", signal);

export const cloneRun = async ({ sourceRunId, fromSnapshotId, newName }, signal) =>
  apiPost(
    "/runs/clone",
    { source_run_id: sourceRunId, from_snapshot_id: fromSnapshotId, new_name: newName },
    "Failed to clone run",
    signal,
  );

export const addRunTasks = async (runId, toml, signal) =>
  apiPost(`/runs/${runId}/tasks`, { toml }, "Failed to add tasks", signal);

export const updateRunTaskQueueTuning = async (runId, taskId, queueTuning, signal) =>
  apiPost(
    `/runs/${runId}/tasks/${taskId}/queue-tuning`,
    { queue_tuning: queueTuning },
    "Failed to update task queue tuning",
    signal,
  );

export const deleteRun = async (runId, signal) => apiDelete(`/runs/${runId}`, "Failed to delete run", signal);

export const deleteRunTask = async (runId, taskId, signal) =>
  apiDelete(`/runs/${runId}/tasks/${taskId}`, "Failed to delete pending task", signal);

export const autoAssignRun = async (runId, { maxEvaluators = null } = {}, signal) =>
  apiPost(`/runs/${runId}/auto-assign`, { max_evaluators: maxEvaluators }, "Failed to auto-assign run", signal);

export const assignNode = async (nodeName, { runId, role }, signal) =>
  apiPost(`/nodes/${nodeName}/assign`, { run_id: runId, role }, "Failed to assign node", signal);

export const unassignNode = async (nodeName, signal) =>
  apiPost(`/nodes/${nodeName}/unassign`, {}, "Failed to unassign node", signal);

export const unassignAllNodes = async (signal) =>
  apiPost("/nodes/unassign-all", {}, "Failed to unassign all nodes", signal);

export const stopNode = async (nodeName, signal) =>
  apiPost(`/nodes/${nodeName}/stop`, {}, "Failed to stop node", signal);

export const stopAllNodes = async (signal) => apiPost("/nodes/stop-all", {}, "Failed to stop all nodes", signal);

export const restartDatabase = async (signal) => apiPost("/admin/db/restart", {}, "Failed to restart database", signal);

export const shutdownControlProcess = async (signal) =>
  apiPost("/admin/control/shutdown", {}, "Failed to shut down control process", signal);

export const autoRunNodes = async ({ toml, count = null, maxStartFailures = null, dbPoolSize = null }, signal) =>
  apiPost(
    "/nodes/auto-run",
    {
      toml,
      count,
      max_start_failures: maxStartFailures,
      db_pool_size: dbPoolSize,
    },
    "Failed to start nodes",
    signal,
  );

export const fetchNodeLaunchRequests = async (signal) => {
  const data = await apiGet("/node-launch-requests", "Failed to fetch node launch requests", signal);
  return data.items;
};

export const fetchNodes = async (runId = null, signal) => {
  return apiGet(`/nodes${buildQueryString([["run_id", runId]])}`, "Failed to fetch nodes", signal);
};

export const fetchNodePanels = async (nodeName, signal) =>
  apiGet(`/nodes/${nodeName}/panels`, "Failed to fetch node panels", signal);

export const fetchRunReproToml = async (runId, signal) =>
  apiGet(`/runs/${runId}/repro-toml`, "Failed to export run TOML", signal);

export const fetchRunPanels = async (runId, signal) =>
  apiGet(`/runs/${runId}/panels`, "Failed to fetch run panels", signal);

export const fetchRunTasks = async (runId, signal) => {
  return apiGet(`/runs/${runId}/tasks`, "Failed to fetch run tasks", signal);
};

export const fetchRunTaskPanels = async (
  runId,
  taskId,
  { limit = 500, cursor = null, panelState = {}, panelActions = [] } = {},
  signal,
) =>
  apiPost(
    `/runs/${runId}/tasks/${taskId}/output`,
    {
      limit,
      cursor,
      panel_state: panelState,
      panel_actions: panelActions,
    },
    "Failed to fetch task panels",
    signal,
  );

export const fetchTemplateList = async (kind, signal) => {
  const data = await apiGet(`/templates/${kind}`, `Failed to fetch ${kind} templates`, signal);
  return data.items;
};

export const fetchTemplateFile = async (kind, name, signal) =>
  apiGet(`/templates/${kind}/${encodeURIComponent(name)}`, `Failed to fetch template ${name}`, signal);

export const saveTemplateFile = async (kind, { name, toml }, signal) =>
  apiPost(`/templates/${kind}`, { name, toml }, `Failed to save template ${name}`, signal);

export const deleteTemplateFile = async (kind, name, signal) =>
  apiDelete(`/templates/${kind}/${encodeURIComponent(name)}`, `Failed to delete template ${name}`, signal);

export const fetchRuntimeLogPage = async (
  {
    limit = 100,
    source = null,
    runId = null,
    nodeName = null,
    nodeUuid = null,
    level = null,
    search = "",
    beforeId = null,
    includeChildren = false,
  } = {},
  signal,
) => {
  return apiGet(
    `/logs${buildQueryString([
      ["limit", limit],
      ["source", source],
      ["run_id", runId],
      ["include_children", includeChildren ? "true" : null],
      ["node_name", nodeName],
      ["node_uuid", nodeUuid],
      ["level", level],
      ["q", search],
      ["before_id", beforeId],
    ])}`,
    "Failed to fetch runtime logs",
    signal,
  );
};

export const fetchRunPerformance = (runId, limit = 500, evaluatorNodeName = null, signal) =>
  apiGet(
    `/runs/${runId}/performance${buildQueryString([
      ["limit", limit],
      ["node_name", evaluatorNodeName],
    ])}`,
    "Failed to fetch run performance",
    signal,
  );
