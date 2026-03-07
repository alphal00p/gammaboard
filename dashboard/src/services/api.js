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
    role: entry.role ?? "unknown",
    implementation: entry.implementation ?? "unknown",
    version: entry.version ?? "",
    status: entry.status ?? "unknown",
    last_seen: entry.last_seen ?? null,
    evaluator_metrics: entry.evaluator_metrics ?? null,
    sampler_metrics: entry.sampler_metrics ?? null,
    evaluator_engine_diagnostics: entry.evaluator_engine_diagnostics ?? null,
    sampler_runtime_metrics: entry.sampler_runtime_metrics ?? null,
    sampler_engine_diagnostics: entry.sampler_engine_diagnostics ?? null,
  };
};

const normalizeRunLogEntry = (entry) => {
  if (!entry || typeof entry !== "object") return null;
  const rawId = entry.id ?? entry.log_id ?? null;
  if (rawId == null) return null;

  const rawRunId = entry.run_id ?? entry.runId ?? null;
  const runId = rawRunId == null ? null : Number(rawRunId);
  const timestamp = entry.ts ?? entry.timestamp ?? entry.created_at ?? null;
  const level = typeof entry.level === "string" ? entry.level.toLowerCase() : "info";

  return {
    id: String(rawId),
    ts: timestamp,
    run_id: runId != null && Number.isFinite(runId) ? runId : null,
    node_id: entry.node_id ?? entry.nodeId ?? null,
    worker_id: entry.worker_id ?? entry.workerId ?? null,
    level,
    message: entry.message ?? "",
    fields: entry.fields ?? {},
  };
};

const normalizeRunLogsPayload = (payload) => {
  const rows = Array.isArray(payload)
    ? payload
    : Array.isArray(payload?.logs)
      ? payload.logs
      : Array.isArray(payload?.items)
        ? payload.items
        : [];
  return rows.map(normalizeRunLogEntry).filter(Boolean);
};

export const fetchRuns = async (signal) => {
  const response = await fetch(`${API_BASE_URL}/runs`, { signal });
  return parseJsonOrThrow(response, "Failed to fetch runs");
};

export const fetchWorkers = async (runId = null, signal) => {
  const params = new URLSearchParams();
  if (runId != null) params.set("run_id", String(runId));
  const suffix = params.toString() ? `?${params.toString()}` : "";
  const response = await fetch(`${API_BASE_URL}/workers${suffix}`, { signal });
  const data = await parseJsonOrThrow(response, "Failed to fetch workers");
  return (Array.isArray(data) ? data : []).map(normalizeWorkerEntry).filter(Boolean);
};

export const fetchStats = async (runId, signal) => {
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/stats`, { signal });
  return parseJsonOrThrow(response, "Failed to fetch stats");
};

export const fetchRun = async (runId, signal) => {
  const response = await fetch(`${API_BASE_URL}/runs/${runId}`, { signal });
  return parseJsonOrThrow(response, "Failed to fetch run");
};

export const fetchRunLogs = async (runId, limit = 500, workerId = null, level = null, signal, afterId = null) => {
  const params = new URLSearchParams({ limit: String(limit) });
  if (workerId) params.set("worker_id", workerId);
  if (level) params.set("level", level);
  if (afterId != null) params.set("after_id", String(afterId));
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/logs?${params.toString()}`, { signal });
  const data = await parseJsonOrThrow(response, "Failed to fetch run logs");
  return normalizeRunLogsPayload(data);
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

export const fetchEvaluatorPerformanceHistory = async (runId, limit = 500, workerId = null, signal) => {
  const params = new URLSearchParams({ limit: String(limit) });
  if (workerId) params.set("worker_id", workerId);
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/performance/evaluator?${params.toString()}`, { signal });
  return parseJsonOrThrow(response, "Failed to fetch evaluator performance history");
};

export const fetchSamplerPerformanceHistory = async (runId, limit = 500, workerId = null, signal) => {
  const params = new URLSearchParams({ limit: String(limit) });
  if (workerId) params.set("worker_id", workerId);
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/performance/sampler-aggregator?${params.toString()}`, {
    signal,
  });
  return parseJsonOrThrow(response, "Failed to fetch sampler performance history");
};

export const fetchWorkerEvaluatorPerformanceHistory = async (workerId, limit = 500, signal) => {
  const params = new URLSearchParams({ limit: String(limit) });
  const response = await fetch(`${API_BASE_URL}/workers/${workerId}/performance/evaluator?${params.toString()}`, {
    signal,
  });
  return parseJsonOrThrow(response, "Failed to fetch worker evaluator performance history");
};

export const fetchWorkerSamplerPerformanceHistory = async (workerId, limit = 500, signal) => {
  const params = new URLSearchParams({ limit: String(limit) });
  const response = await fetch(
    `${API_BASE_URL}/workers/${workerId}/performance/sampler-aggregator?${params.toString()}`,
    { signal },
  );
  return parseJsonOrThrow(response, "Failed to fetch worker sampler performance history");
};
