const API_BASE_URL = process.env.REACT_APP_API_BASE_URL;

if (!API_BASE_URL || !API_BASE_URL.trim()) {
  throw new Error("Missing REACT_APP_API_BASE_URL");
}

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
    if (text.trim()) return text.trim();
  } catch {
    // Fall through to status fallback.
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
  const response = await fetch(`${API_BASE_URL}${path}`, { signal });
  return parseJsonOrThrow(response, message);
};

const normalizeWorkerEntry = (entry) => {
  if (!entry || typeof entry !== "object") return null;
  return {
    worker_id: entry.worker_id ?? "",
    node_id: entry.node_id ?? null,
    desired_run_id: Number.isFinite(Number(entry.desired_run_id)) ? Number(entry.desired_run_id) : null,
    desired_role: entry.desired_role ?? null,
    current_run_id: Number.isFinite(Number(entry.current_run_id)) ? Number(entry.current_run_id) : null,
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

const normalizeRunLogEntry = (entry) => {
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
    run_id: runId != null && Number.isFinite(runId) ? runId : null,
    node_id: entry.node_id ?? null,
    worker_id: entry.worker_id ?? null,
    level,
    message: entry.message ?? "",
    fields: entry.fields ?? {},
  };
};

const normalizeRunLogPage = (payload) => {
  const rows = Array.isArray(payload?.items) ? payload.items : [];
  return {
    items: rows.map(normalizeRunLogEntry).filter(Boolean),
    next_before_id: payload?.next_before_id != null ? String(payload.next_before_id) : null,
    has_more_older: payload?.has_more_older === true,
  };
};

const normalizeRunEntry = (entry) => {
  if (!entry || typeof entry !== "object") return null;
  const runId = Number(entry.run_id);
  return {
    ...entry,
    run_id: Number.isFinite(runId) ? runId : entry.run_id,
    nr_produced_samples: Number.isFinite(Number(entry.nr_produced_samples)) ? Number(entry.nr_produced_samples) : 0,
    nr_completed_samples: Number.isFinite(Number(entry.nr_completed_samples)) ? Number(entry.nr_completed_samples) : 0,
    integration_params: entry.integration_params ?? {},
    point_spec: entry.point_spec ?? null,
    target: entry.target ?? null,
  };
};

const normalizeRunTaskEntry = (entry) => {
  if (!entry || typeof entry !== "object") return null;
  return {
    ...entry,
    id: Number.isFinite(Number(entry.id)) ? Number(entry.id) : entry.id,
    run_id: Number.isFinite(Number(entry.run_id)) ? Number(entry.run_id) : entry.run_id,
    sequence_nr: Number.isFinite(Number(entry.sequence_nr)) ? Number(entry.sequence_nr) : entry.sequence_nr,
    nr_produced_samples: Number.isFinite(Number(entry.nr_produced_samples)) ? Number(entry.nr_produced_samples) : 0,
    nr_completed_samples: Number.isFinite(Number(entry.nr_completed_samples)) ? Number(entry.nr_completed_samples) : 0,
  };
};

export const fetchRuns = async (signal) => {
  const data = await apiGet("/runs", "Failed to fetch runs", signal);
  return (Array.isArray(data) ? data : []).map(normalizeRunEntry).filter(Boolean);
};

export const fetchNodes = async (runId = null, signal) => {
  const data = await apiGet(`/nodes${buildQueryString([["run_id", runId]])}`, "Failed to fetch nodes", signal);
  return (Array.isArray(data) ? data : []).map(normalizeWorkerEntry).filter(Boolean);
};

export const fetchStats = async (runId, signal) => {
  return apiGet(`/runs/${runId}/stats`, "Failed to fetch stats", signal);
};

export const fetchRun = async (runId, signal) => {
  const data = await apiGet(`/runs/${runId}`, "Failed to fetch run", signal);
  return normalizeRunEntry(data) ?? data;
};

export const fetchRunTasks = async (runId, signal) => {
  const data = await apiGet(`/runs/${runId}/tasks`, "Failed to fetch run tasks", signal);
  return (Array.isArray(data) ? data : []).map(normalizeRunTaskEntry).filter(Boolean);
};

export const fetchRunTaskOutput = async (runId, taskId, signal) =>
  apiGet(`/runs/${runId}/tasks/${taskId}/output`, "Failed to fetch task output", signal);

export const fetchRunTaskOutputHistory = async (
  runId,
  taskId,
  { limit = 500, afterSnapshotId = null } = {},
  signal,
) => {
  return apiGet(
    `/runs/${runId}/tasks/${taskId}/output/history${buildQueryString([
      ["limit", limit],
      ["after_snapshot_id", afterSnapshotId],
    ])}`,
    "Failed to fetch task output history",
    signal,
  );
};

export const fetchRunLogPage = async (
  runId,
  { limit = 100, nodeId = null, level = null, search = "", beforeId = null } = {},
  signal,
) => {
  const data = await apiGet(
    `/runs/${runId}/logs${buildQueryString([
      ["limit", limit],
      ["node_id", nodeId],
      ["level", level],
      ["q", search],
      ["before_id", beforeId],
    ])}`,
    "Failed to fetch run logs",
    signal,
  );
  return normalizeRunLogPage(data);
};

export const fetchEvaluatorPerformanceHistory = async (runId, limit = 500, nodeId = null, signal) => {
  return apiGet(
    `/runs/${runId}/performance/evaluator${buildQueryString([
      ["limit", limit],
      ["node_id", nodeId],
    ])}`,
    "Failed to fetch evaluator performance history",
    signal,
  );
};

export const fetchSamplerPerformanceHistory = async (runId, limit = 500, nodeId = null, signal) => {
  return apiGet(
    `/runs/${runId}/performance/sampler-aggregator${buildQueryString([
      ["limit", limit],
      ["node_id", nodeId],
    ])}`,
    "Failed to fetch sampler performance history",
    signal,
  );
};

export const fetchNodeEvaluatorPerformanceHistory = async (nodeId, limit = 500, signal) =>
  apiGet(
    `/nodes/${nodeId}/performance/evaluator${buildQueryString([["limit", limit]])}`,
    "Failed to fetch node evaluator performance history",
    signal,
  );

export const fetchNodeSamplerPerformanceHistory = async (nodeId, limit = 500, signal) =>
  apiGet(
    `/nodes/${nodeId}/performance/sampler-aggregator${buildQueryString([["limit", limit]])}`,
    "Failed to fetch node sampler performance history",
    signal,
  );
