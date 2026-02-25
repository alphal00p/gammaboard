const API_BASE_URL = process.env.REACT_APP_API_BASE_URL || "http://localhost:4000/api";

const parseJsonOrThrow = async (response, message) => {
  if (!response.ok) throw new Error(`${message}: ${response.statusText}`);
  return response.json();
};

export const fetchRuns = async () => {
  const response = await fetch(`${API_BASE_URL}/runs`);
  return parseJsonOrThrow(response, "Failed to fetch runs");
};

export const fetchWorkers = async (runId = null) => {
  const params = new URLSearchParams();
  if (runId != null) params.set("run_id", String(runId));
  const suffix = params.toString() ? `?${params.toString()}` : "";
  const response = await fetch(`${API_BASE_URL}/workers${suffix}`);
  return parseJsonOrThrow(response, "Failed to fetch workers");
};

export const fetchStats = async (runId) => {
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/stats`);
  return parseJsonOrThrow(response, "Failed to fetch stats");
};

export const fetchRun = async (runId) => {
  const response = await fetch(`${API_BASE_URL}/runs/${runId}`);
  return parseJsonOrThrow(response, "Failed to fetch run");
};

export const fetchRunLogs = async (runId, limit = 500, workerId = null, level = null) => {
  const params = new URLSearchParams({ limit: String(limit) });
  if (workerId) params.set("worker_id", workerId);
  if (level) params.set("level", level);
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/logs?${params.toString()}`);
  return parseJsonOrThrow(response, "Failed to fetch run logs");
};

export const fetchAggregatedHistory = async (runId, limit) => {
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/aggregated?limit=${limit}`);
  return parseJsonOrThrow(response, "Failed to fetch aggregated history");
};

export const fetchLatestAggregated = async (runId) => {
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/aggregated/latest`);
  if (response.status === 404) return null;
  return parseJsonOrThrow(response, "Failed to fetch latest aggregated result");
};

export const fetchEvaluatorPerformanceHistory = async (runId, limit = 500, workerId = null) => {
  const params = new URLSearchParams({ limit: String(limit) });
  if (workerId) params.set("worker_id", workerId);
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/performance/evaluator?${params.toString()}`);
  return parseJsonOrThrow(response, "Failed to fetch evaluator performance history");
};

export const fetchSamplerPerformanceHistory = async (runId, limit = 500, workerId = null) => {
  const params = new URLSearchParams({ limit: String(limit) });
  if (workerId) params.set("worker_id", workerId);
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/performance/sampler-aggregator?${params.toString()}`);
  return parseJsonOrThrow(response, "Failed to fetch sampler performance history");
};

export const createRunStatsEventSource = (runId, intervalMs) =>
  new EventSource(`${API_BASE_URL}/runs/${runId}/stream?interval_ms=${intervalMs}`);
