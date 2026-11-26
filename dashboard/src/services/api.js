import { clearResourceCache } from "./resourceCache";
const APP_BASE_URL = import.meta.env.BASE_URL || "/";
const normalizedAppBase = APP_BASE_URL.endsWith("/") ? APP_BASE_URL : `${APP_BASE_URL}/`;
const API_BASE_URL = `${normalizedAppBase}api`;
export const apiUrl = (path = "") => `${API_BASE_URL}${path.startsWith("/") ? path : `/${path}`}`;

const stripHtml = (value) =>
  value
    .replace(/<[^>]*>/g, " ")
    .replace(/\s+/g, " ")
    .trim();

const gatewayError = (status) => {
  if (status === 504) return "gateway timeout (504): the request took too long and may still be running";
  if (status === 502 || status === 503) return `gateway error (${status}): backend unavailable or restarting`;
  return null;
};

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
        return gatewayError(response.status) || summary || `HTTP ${response.status}`;
      }
      return text.trim();
    }
  } catch {
    // Fall through to status fallback.
  }

  return gatewayError(response.status) || response.statusText || `HTTP ${response.status}`;
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

const manage = (payload, message, signal) => apiPost("/v1/manage", payload, message, signal);
const taskAction = (runId, action, message, signal) =>
  manage({ operation: "task", run: String(runId), action }, message, signal);

export const importHwuBundle = (contents, signal) =>
  apiRequest("/histogram-bundle/import", "Failed to import HwU bundle", signal, {
    method: "POST", headers: { "content-type": "text/plain;charset=UTF-8" }, body: contents,
  });

export const fetchRuns = async ({ includeChildren = false, limit = 100, offset = 0, search = "" } = {}, signal) => {
  const data = await apiGet(
    `/runs${buildQueryString([
      ["include_children", includeChildren ? "true" : null],
      ["limit", limit],
      ["offset", offset],
      ["search", search],
    ])}`,
    "Failed to fetch runs",
    signal,
  );
  return {
    items: data.items,
    nextOffset: data.next_offset,
  };
};

export const fetchRunSummary = (runId, signal) => apiGet(`/runs/${runId}`, "Failed to fetch run", signal);

export const fetchServerStatus = async (signal) => apiGet("/health", "Failed to fetch server status", signal);

// POST makes browsers include Origin even through the same-origin dashboard
// proxy, so a forwarded port mismatch is detected before mounting workspaces.
export const fetchSession = async (signal) => apiPost("/auth/session", {}, "Failed to check browser access", signal);

export const fetchSettingsOverview = async (signal) => apiGet("/settings", "Failed to fetch settings", signal);

export const login = async (password, signal) => { clearResourceCache(); return apiPost("/auth/login", { password }, "Failed to log in", signal); };

export const logout = async (signal) => { clearResourceCache(); return apiPost("/auth/logout", {}, "Failed to log out", signal); };

export const pauseRun = (runId, signal) =>
  manage({ operation: "run_pause", runs: [String(runId)], all: false }, "Failed to pause run", signal);

export const createRun = async (toml, { duplicate = false, signal } = {}) => {
  const settings = await fetchSettingsOverview(signal);
  const receipt = await manage({ operation: "run_create", toml, duplicate, start: false, base_dir: settings.paths.run_templates_dir }, "Failed to create run", signal);
  return receipt.created;
};

export const addRunTasks = (runId, toml, { duplicate = false, signal } = {}) =>
  taskAction(runId, { action: "append", toml, duplicate }, "Failed to add tasks", signal);

export const updateRunTaskQueueTuning = (runId, taskId, queueTuning, signal) =>
  taskAction(runId, { action: "tune", id: String(taskId), queue_tuning: queueTuning }, "Failed to update task queue tuning", signal);

const waitForRemovalPoll = (delay, signal) => new Promise((resolve, reject) => {
  signal?.throwIfAborted();
  const abort = () => {
    clearTimeout(timer);
    reject(signal.reason);
  };
  const timer = setTimeout(() => {
    signal?.removeEventListener("abort", abort);
    resolve();
  }, delay);
  signal?.addEventListener("abort", abort, { once: true });
});

export const deleteRun = async (runId, signal) => {
  const receipt = await manage({ operation: "run_remove", runs: [String(runId)], all: false, dry_run: false }, "Failed to delete run", signal);
  let operation = receipt.operations[0];
  while (operation.status === "running") {
    await waitForRemovalPoll(operation.poll_after_ms ?? 1000, signal);
    operation = await manage(
      { operation: "run_removal_status", operation_id: operation.operation_id },
      "Unable to check deletion status; refresh the run list before retrying",
      signal,
    );
  }
  if (operation.status !== "completed") {
    throw new Error(`Failed to delete run: ${operation.error || "deletion did not complete"}`);
  }
  clearResourceCache();
  return operation;
};

export const deleteRunTask = async (runId, taskId, signal) => {
  const result = await taskAction(runId, { action: "remove", id: String(taskId) }, "Failed to delete pending task", signal);
  clearResourceCache(); return result;
};

export const autoAssignRun = async (runId, { maxEvaluators = null } = {}, signal) => {
  const receipt = await manage({ operation: "run_start", run: String(runId), max_evaluators: maxEvaluators }, "Failed to start run", signal);
  if (receipt.start.state !== "started") throw new Error(receipt.start.error?.message || `Run start ${receipt.start.state}`);
  return receipt;
};

export const assignNode = (nodeName, { runId, role }, signal) =>
  manage({ operation: "worker_assign", name: nodeName, run: String(runId), role }, "Failed to assign worker", signal);
export const unassignNode = (nodeName, signal) =>
  manage({ operation: "worker_unassign", name: nodeName }, "Failed to unassign worker", signal);
export const unassignAllNodes = (signal) =>
  manage({ operation: "worker_unassign", all: true }, "Failed to unassign all workers", signal);
export const stopNode = (nodeName, signal) =>
  manage({ operation: "worker_stop", names: [nodeName], all: false, idle: false }, "Failed to stop worker", signal);
export const stopAllNodes = (signal) =>
  manage({ operation: "worker_stop", names: [], all: true, idle: false }, "Failed to stop all workers", signal);
export const stopIdleNodes = (signal) =>
  manage({ operation: "worker_stop", names: [], all: false, idle: true }, "Failed to stop idle workers", signal);
export const fetchOfflineNodes = (signal) => apiGet("/workers?state=offline", "Failed to load offline workers", signal);
export const updateOfflineNode = (node, action) => {
  const identity = { name: node.node_name, expected_uuid: node.node_uuid };
  const payload = action === "start" || action === "restore"
    ? { operation: "worker_launch", source: { kind: "saved", ...identity, restore_only: action === "restore" } }
    : { operation: action === "unassign-offline" ? "worker_unassign" : "worker_cancel_restoration", ...identity };
  return manage(payload, `Failed to ${action} for ${node.node_name}`);
};

export const shutdownControlProcess = (signal) =>
  manage({ operation: "stop", wait: false, timeout_ms: null }, "Failed to stop instance", signal);

export const autoRunNodes = ({ toml, count = null, maxStartFailures = null }, signal) =>
  manage({ operation: "worker_launch", source: { kind: "new", toml, count, max_start_failures: maxStartFailures } }, "Failed to request workers", signal);
export const fetchNodeLaunchRequests = (signal) =>
  manage({ operation: "worker_requests", id: null }, "Failed to fetch worker launch requests", signal);
export const fetchNodes = (runId = null, signal) =>
  apiGet(`/workers${buildQueryString([["state", "live"], ["run_id", runId]])}`, "Failed to fetch workers", signal);
export const fetchNodePanels = (nodeName, signal) =>
  apiGet(`/workers/${encodeURIComponent(nodeName)}/panels`, "Failed to fetch worker panels", signal);

export const fetchRunDefinition = async (runId, duplicate = false, signal) =>
  apiGet(`/runs/${runId}/definition?duplicate=${duplicate}`, "Failed to load run definition", signal);

export const fetchTaskDefinition = async (runId, taskId, duplicate = false, signal) =>
  apiGet(`/runs/${runId}/tasks/${taskId}/definition?duplicate=${duplicate}`, "Failed to load task definition", signal);

export const editRunTask = (runId, taskId, toml, expectedToml, signal) =>
  taskAction(runId, { action: "edit", id: String(taskId), toml, expected_toml: expectedToml }, "Failed to edit task", signal);

export const fetchRunPanels = async (runId, signal) =>
  apiGet(`/runs/${runId}/panels`, "Failed to fetch run panels", signal);

export const fetchRunTasks = async (runId, signal) => {
  return apiGet(`/runs/${runId}/tasks`, "Failed to fetch run tasks", signal);
};

export const fetchRunTaskPanels = async (
  runId,
  taskId,
  { limit = 256, cursor = null, panelState = {}, panelActions = [], section, phase } = {},
  signal,
) =>
  apiPost(
    `/runs/${runId}/tasks/${taskId}/output`,
    {
      limit,
      cursor,
      section,
      phase,
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
  manage({ operation: "template", kind: ({ runs: "run", tasks: "task", nodes: "worker" })[kind] || kind, action: { action: "save", name, toml, overwrite: true } }, `Failed to save template ${name}`, signal);

export const deleteTemplateFile = async (kind, name, signal) =>
  manage({ operation: "template", kind: ({ runs: "run", tasks: "task", nodes: "worker" })[kind] || kind, action: { action: "remove", name } }, `Failed to delete template ${name}`, signal);

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

const performanceRangeQuery = (selection) => selection?.seconds != null || selection?.follow
  ? [["window_seconds", selection.seconds ?? (selection.end - selection.start) / 1000]]
  : [["start_ms", selection?.start], ["end_ms", selection?.end]];

export const fetchRunPerformanceGraphs = (runId, selection = null, signal) =>
  apiGet(
    `/runs/${runId}/performance/graphs${buildQueryString([
      ...performanceRangeQuery(selection),
    ])}`,
    "Failed to fetch performance history",
    signal,
  );

export const fetchRunPerformance = (runId, selection = null, evaluatorNodeName = null, signal) =>
  apiGet(
    `/runs/${runId}/performance${buildQueryString([
      ...performanceRangeQuery(selection),
      ["node_name", evaluatorNodeName],
    ])}`,
    "Failed to fetch run performance",
    signal,
  );

export const fetchCampaign = (runId, taskId, { section = "settings", group, selection } = {}, signal) =>
  apiGet(`/runs/${runId}/tasks/${taskId}/campaign${buildQueryString([
    ["section", section], ["group", group], ...performanceRangeQuery(selection),
  ])}`, "Failed to fetch campaign", signal);
export const updateCampaignScheduling = (runId, taskId, payload) => taskAction(runId, { action: "campaign_scheduling", id: String(taskId), settings: payload }, "Failed to update campaign scheduling");
export const updateCampaignTuning = (runId, taskId, payload) => taskAction(runId, { action: "campaign_tuning", id: String(taskId), settings: payload }, "Failed to update campaign queue tuning");
