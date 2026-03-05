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
    throw new Error(`${message}: ${details}`);
  }
  return response.json();
};

export const fetchRuns = async (signal) => {
  const response = await fetch(`${API_BASE_URL}/runs`, { signal });
  return parseJsonOrThrow(response, "Failed to fetch runs");
};

export const fetchWorkers = async (runId = null) => {
  const params = new URLSearchParams();
  if (runId != null) params.set("run_id", String(runId));
  const suffix = params.toString() ? `?${params.toString()}` : "";
  const response = await fetch(`${API_BASE_URL}/workers${suffix}`);
  return parseJsonOrThrow(response, "Failed to fetch workers");
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
  return parseJsonOrThrow(response, "Failed to fetch run logs");
};

export const fetchAggregatedHistory = async (runId, limit, signal) => {
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/aggregated?limit=${limit}`, { signal });
  return parseJsonOrThrow(response, "Failed to fetch aggregated history");
};

export const fetchAggregatedRange = async (runId, start, stop, step, latestId = null, signal) => {
  const params = new URLSearchParams({
    start: String(start),
    stop: String(stop),
    step: String(step),
  });
  if (latestId != null) params.set("latest_id", String(latestId));
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/aggregated/range?${params.toString()}`, { signal });
  return parseJsonOrThrow(response, "Failed to fetch aggregated range");
};

export const fetchLatestAggregated = async (runId, signal) => {
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/aggregated/latest`, { signal });
  if (response.status === 404) return null;
  return parseJsonOrThrow(response, "Failed to fetch latest aggregated result");
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
