import { asArray } from "../utils/collections";

const API_BASE_URL = "/api";

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

const apiGet = async (path, message, signal) => {
  const response = await fetch(`${API_BASE_URL}${path}`, {
    credentials: "include",
    signal,
  });
  return parseJsonOrThrow(response, message);
};

const apiPost = async (path, payload, message, signal) => {
  const response = await fetch(`${API_BASE_URL}${path}`, {
    method: "POST",
    credentials: "include",
    headers: {
      "content-type": "application/json",
    },
    body: JSON.stringify(payload ?? {}),
    signal,
  });
  return parseJsonOrThrow(response, message);
};

const apiDelete = async (path, message, signal) => {
  const response = await fetch(`${API_BASE_URL}${path}`, {
    method: "DELETE",
    credentials: "include",
    signal,
  });
  return parseJsonOrThrow(response, message);
};

const normalizeWorkerEntry = (entry) => {
  if (!entry || typeof entry !== "object") return null;
  return {
    node_name: entry.node_name ?? "",
    node_uuid: entry.node_uuid ?? "",
    capabilities: entry.capabilities && typeof entry.capabilities === "object" ? entry.capabilities : {},
    desired_run_id: Number.isFinite(Number(entry.desired_run_id)) ? Number(entry.desired_run_id) : null,
    desired_run_name: entry.desired_run_name ?? null,
    desired_role: entry.desired_role ?? null,
    current_run_id: Number.isFinite(Number(entry.current_run_id)) ? Number(entry.current_run_id) : null,
    current_run_name: entry.current_run_name ?? null,
    current_role: entry.current_role ?? null,
    role: entry.role ?? "unknown",
    implementation: entry.implementation ?? "unknown",
    version: entry.version ?? "",
    status: entry.status ?? "unknown",
    last_seen: entry.last_seen ?? null,
    evaluator_metrics: entry.evaluator_metrics ?? null,
    sampler_metrics: entry.sampler_metrics ?? null,
    sampler_runtime_metrics: entry.sampler_runtime_metrics ?? null,
    sampler_engine_diagnostics: entry.sampler_engine_diagnostics ?? null,
  };
};

const normalizeRuntimeLogEntry = (entry) => {
  if (!entry || typeof entry !== "object") return null;
  const rawId = entry.id ?? null;
  if (rawId == null) return null;

  const rawRunId = entry.run_id ?? null;
  const runId = rawRunId == null ? null : Number(rawRunId);
  const timestamp = entry.ts ?? null;
  const level = typeof entry.level === "string" ? entry.level.toLowerCase() : "info";

  return {
    id: String(rawId),
    ts: timestamp,
    source: typeof entry.source === "string" ? entry.source : "unknown",
    run_id: runId != null && Number.isFinite(runId) ? runId : null,
    node_uuid: entry.node_uuid ?? null,
    node_name: entry.node_name ?? null,
    level,
    target: typeof entry.target === "string" ? entry.target : "",
    message: entry.message ?? "",
    fields: entry.fields ?? {},
  };
};

const normalizeRuntimeLogPage = (payload) => {
  const rows = asArray(payload?.items);
  return {
    items: rows.map(normalizeRuntimeLogEntry).filter(Boolean),
    next_before_id: payload?.next_before_id != null ? String(payload.next_before_id) : null,
    has_more_older: payload?.has_more_older === true,
  };
};

const normalizeRunEntry = (entry) => {
  if (!entry || typeof entry !== "object") return null;
  const runId = Number(entry.run_id);
  const rootStageSnapshotId = entry.root_stage_snapshot_id == null ? null : String(entry.root_stage_snapshot_id);
  return {
    ...entry,
    run_id: Number.isFinite(runId) ? runId : entry.run_id,
    root_stage_snapshot_id: rootStageSnapshotId,
    nr_produced_samples: Number.isFinite(Number(entry.nr_produced_samples)) ? Number(entry.nr_produced_samples) : 0,
    nr_completed_samples: Number.isFinite(Number(entry.nr_completed_samples)) ? Number(entry.nr_completed_samples) : 0,
    sampler_runner_uptime_ms: Number.isFinite(Number(entry.sampler_runner_uptime_ms))
      ? Number(entry.sampler_runner_uptime_ms)
      : 0,
    integration_params: entry.integration_params ?? {},
    point_spec: entry.point_spec ?? null,
    target: entry.target ?? null,
  };
};

const normalizeRunTaskEntry = (entry) => {
  if (!entry || typeof entry !== "object") return null;
  if (entry.id == null) return null;
  return {
    ...entry,
    id: entry.id == null ? null : String(entry.id),
    run_id: Number.isFinite(Number(entry.run_id)) ? Number(entry.run_id) : entry.run_id,
    name: typeof entry.name === "string" ? entry.name : String(entry.name ?? ""),
    latest_stage_snapshot_id: entry.latest_stage_snapshot_id == null ? null : String(entry.latest_stage_snapshot_id),
    root_stage_snapshot_id: entry.root_stage_snapshot_id == null ? null : String(entry.root_stage_snapshot_id),
    nr_produced_samples: Number.isFinite(Number(entry.nr_produced_samples)) ? Number(entry.nr_produced_samples) : 0,
    nr_completed_samples: Number.isFinite(Number(entry.nr_completed_samples)) ? Number(entry.nr_completed_samples) : 0,
  };
};

export const fetchRuns = async (signal) => {
  const data = await apiGet("/runs", "Failed to fetch runs", signal);
  return asArray(data).map(normalizeRunEntry).filter(Boolean);
};

export const fetchSession = async (signal) => apiGet("/auth/session", "Failed to fetch session", signal);

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

export const stopNode = async (nodeName, signal) =>
  apiPost(`/nodes/${nodeName}/stop`, {}, "Failed to stop node", signal);

export const stopAllNodes = async (signal) => apiPost("/nodes/stop-all", {}, "Failed to stop all nodes", signal);

export const restartDatabase = async (signal) => apiPost("/admin/db/restart", {}, "Failed to restart database", signal);

export const shutdownControlProcess = async (signal) =>
  apiPost("/admin/control/shutdown", {}, "Failed to shut down control process", signal);

export const autoRunNodes = async ({ count, maxStartFailures = null, dbPoolSize = null }, signal) =>
  apiPost(
    "/nodes/auto-run",
    {
      count,
      max_start_failures: maxStartFailures,
      db_pool_size: dbPoolSize,
    },
    "Failed to start nodes",
    signal,
  );

export const fetchNodeLaunchRequests = async (signal) => {
  const data = await apiGet("/node-launch-requests", "Failed to fetch node launch requests", signal);
  return asArray(data?.items).map((entry) => ({
    ...entry,
    id: entry?.id == null ? "" : String(entry.id),
    requested_count: Number.isFinite(Number(entry?.requested_count)) ? Number(entry.requested_count) : 0,
    started_count: Number.isFinite(Number(entry?.started_count)) ? Number(entry.started_count) : 0,
    args: entry?.args ?? {},
    result: entry?.result ?? {},
  }));
};

export const fetchNodes = async (runId = null, signal) => {
  const data = await apiGet(`/nodes${buildQueryString([["run_id", runId]])}`, "Failed to fetch nodes", signal);
  return asArray(data).map(normalizeWorkerEntry).filter(Boolean);
};

export const fetchNodePanels = async (nodeName, signal) =>
  apiGet(`/nodes/${nodeName}/panels`, "Failed to fetch node panels", signal);

export const fetchRunReproToml = async (runId, signal) =>
  apiGet(`/runs/${runId}/repro-toml`, "Failed to export run TOML", signal);

export const fetchRunPanels = async (runId, signal) =>
  apiGet(`/runs/${runId}/panels`, "Failed to fetch run panels", signal);

export const fetchRunDebugBatches = async (runId, { limit = 1000, status = "claimed" } = {}, signal) =>
  apiGet(
    `/runs/${runId}/debug/batches${buildQueryString([
      ["limit", limit],
      ["status", status],
    ])}`,
    "Failed to fetch debug batches",
    signal,
  );

export const fetchRunTasks = async (runId, signal) => {
  const data = await apiGet(`/runs/${runId}/tasks`, "Failed to fetch run tasks", signal);
  return asArray(data).map(normalizeRunTaskEntry).filter(Boolean);
};

export const fetchRunEvaluatorConfigPanels = async (runId, signal) =>
  apiGet(`/runs/${runId}/config/evaluator`, "Failed to fetch evaluator config panels", signal);

export const fetchRunSamplerConfigPanels = async (runId, signal) =>
  apiGet(`/runs/${runId}/config/sampler-aggregator`, "Failed to fetch sampler config panels", signal);

export const fetchRunTaskPanels = async (
  runId,
  taskId,
  { limit = 500, cursor = null, panelState = {}, panelActions = [] } = {},
  signal,
) => {
  return apiPost(
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
};

export const fetchTemplateList = async (kind, signal) => {
  const data = await apiGet(`/templates/${kind}`, `Failed to fetch ${kind} templates`, signal);
  return asArray(data?.items).filter((value) => typeof value === "string" && value.trim());
};

export const fetchTemplateFile = async (kind, name, signal) => {
  return apiGet(`/templates/${kind}/${encodeURIComponent(name)}`, `Failed to fetch template ${name}`, signal);
};

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
  } = {},
  signal,
) => {
  const data = await apiGet(
    `/logs${buildQueryString([
      ["limit", limit],
      ["source", source],
      ["run_id", runId],
      ["node_name", nodeName],
      ["node_uuid", nodeUuid],
      ["level", level],
      ["q", search],
      ["before_id", beforeId],
    ])}`,
    "Failed to fetch runtime logs",
    signal,
  );
  return normalizeRuntimeLogPage(data);
};

export const fetchRunLogPage = async (
  runId,
  { limit = 100, source = null, nodeName = null, nodeUuid = null, level = null, search = "", beforeId = null } = {},
  signal,
) =>
  fetchRuntimeLogPage(
    {
      limit,
      source,
      runId,
      nodeName,
      nodeUuid,
      level,
      search,
      beforeId,
    },
    signal,
  );

export const fetchEvaluatorPerformanceHistory = async (runId, limit = 500, nodeName = null, signal) => {
  return apiGet(
    `/runs/${runId}/performance/evaluator${buildQueryString([
      ["limit", limit],
      ["node_name", nodeName],
    ])}`,
    "Failed to fetch evaluator performance history",
    signal,
  );
};

export const fetchSamplerPerformanceHistory = async (runId, limit = 500, nodeName = null, signal) => {
  return apiGet(
    `/runs/${runId}/performance/sampler-aggregator${buildQueryString([
      ["limit", limit],
      ["node_name", nodeName],
    ])}`,
    "Failed to fetch sampler performance history",
    signal,
  );
};

export const fetchNodeEvaluatorPerformanceHistory = async (nodeName, limit = 500, signal) =>
  apiGet(
    `/nodes/${nodeName}/performance/evaluator${buildQueryString([["limit", limit]])}`,
    "Failed to fetch node evaluator performance history",
    signal,
  );

export const fetchNodeSamplerPerformanceHistory = async (nodeName, limit = 500, signal) =>
  apiGet(
    `/nodes/${nodeName}/performance/sampler-aggregator${buildQueryString([["limit", limit]])}`,
    "Failed to fetch node sampler performance history",
    signal,
  );
