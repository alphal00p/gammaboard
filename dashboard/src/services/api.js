const API_BASE_URL = "http://localhost:4000/api";

/**
 * Fetches all runs from the backend
 * @returns {Promise<Array>} List of runs
 */
export const fetchRuns = async () => {
  const response = await fetch(`${API_BASE_URL}/runs`);
  if (!response.ok) {
    throw new Error(`Failed to fetch runs: ${response.statusText}`);
  }
  return response.json();
};

/**
 * Fetches samples for a specific run
 * @param {number} runId - The run ID
 * @param {number} limit - Maximum number of samples to fetch
 * @returns {Promise<Array>} List of samples
 */
export const fetchSamples = async (runId, limit = 500) => {
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/samples?limit=${limit}`);
  if (!response.ok) {
    throw new Error(`Failed to fetch samples: ${response.statusText}`);
  }
  return response.json();
};

/**
 * Fetches statistics for a specific run
 * @param {number} runId - The run ID
 * @returns {Promise<Array>} List of statistics
 */
export const fetchStats = async (runId) => {
  const response = await fetch(`${API_BASE_URL}/runs/${runId}/stats`);
  if (!response.ok) {
    throw new Error(`Failed to fetch stats: ${response.statusText}`);
  }
  return response.json();
};
