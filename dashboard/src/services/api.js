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
  const response = await fetch(`${API_BASE_URL}/runs`, { signal });
  const data = await parseJsonOrThrow(response, "Failed to fetch runs");
  return (Array.isArray(data) ? data : []).map(normalizeRunEntry).filter(Boolean);
};

export const fetchNodes = async (runId = null, signal) => {
  const params = new URLSearchParams();
  if (runId != null) params.set("run_id", String(runId));
  const suffix = params.toString() ? `?${params.toString()}` : "";
  const response = await fetch(`${API_BASE_URL}/nodes${suffix}`, { signal });
  const data = await parseJsonOrThrow(response, "Failed to fetch nodes");
  return (Array.isArray(data) ? data : []).map(normalizeWorkerEntry).filter(Boolean);
};

export const fetchStats = async (runId, signal) => {
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/stats`, { signal });
  return parseJsonOrThrow(response, "Failed to fetch stats");
};

export const fetchRun = async (runId, signal) => {
  const response = await fetch(`${API_BASE_URL}/runs/${runId}`, { signal });
  const data = await parseJsonOrThrow(response, "Failed to fetch run");
  return normalizeRunEntry(data) ?? data;
};

export const fetchRunTasks = async (runId, signal) => {
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/tasks`, { signal });
  const data = await parseJsonOrThrow(response, "Failed to fetch run tasks");
  return (Array.isArray(data) ? data : []).map(normalizeRunTaskEntry).filter(Boolean);
};

export const fetchRunLogPage = async (
  runId,
  { limit = 100, nodeId = null, level = null, search = "", beforeId = null } = {},
  signal,
) => {
  const params = new URLSearchParams({ limit: String(limit) });
  if (nodeId) params.set("node_id", nodeId);
  if (level) params.set("level", level);
  if (search && search.trim()) params.set("q", search.trim());
  if (beforeId != null) params.set("before_id", String(beforeId));
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/logs?${params.toString()}`, { signal });
  const data = await parseJsonOrThrow(response, "Failed to fetch run logs");
  return normalizeRunLogPage(data);
};

export const fetchAggregatedRange = async (runId, start, stop, maxPoints, lastId = null, signal) => {
  const params = new URLSearchParams({
    start: String(start),
    stop: String(stop),
    max_points: String(maxPoints),
  });
  if (lastId != null) params.set("last_id", String(lastId));
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/aggregated/range?${params.toString()}`, { signal });
  return parseJsonOrThrow(response, "Failed to fetch aggregated range");
};

export const fetchEvaluatorPerformanceHistory = async (runId, limit = 500, nodeId = null, signal) => {
  const params = new URLSearchParams({ limit: String(limit) });
  if (nodeId) params.set("node_id", nodeId);
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/performance/evaluator?${params.toString()}`, { signal });
  return parseJsonOrThrow(response, "Failed to fetch evaluator performance history");
};

export const fetchSamplerPerformanceHistory = async (runId, limit = 500, nodeId = null, signal) => {
  const params = new URLSearchParams({ limit: String(limit) });
  if (nodeId) params.set("node_id", nodeId);
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/performance/sampler-aggregator?${params.toString()}`, {
    signal,
  });
  return parseJsonOrThrow(response, "Failed to fetch sampler performance history");
};

export const fetchNodeEvaluatorPerformanceHistory = async (nodeId, limit = 500, signal) => {
  const params = new URLSearchParams({ limit: String(limit) });
  const response = await fetch(`${API_BASE_URL}/nodes/${nodeId}/performance/evaluator?${params.toString()}`, {
    signal,
  });
  return parseJsonOrThrow(response, "Failed to fetch node evaluator performance history");
};

export const fetchNodeSamplerPerformanceHistory = async (nodeId, limit = 500, signal) => {
  const params = new URLSearchParams({ limit: String(limit) });
  const response = await fetch(`${API_BASE_URL}/nodes/${nodeId}/performance/sampler-aggregator?${params.toString()}`, {
    signal,
  });
  return parseJsonOrThrow(response, "Failed to fetch node sampler performance history");
};
