export const formatRunLabel = (run) => {
  if (!run) return "Unknown run";
  return run.run_name ? run.run_name : "Unnamed run";
};

export const deriveRunLifecycle = (run) => {
  if (!run || typeof run !== "object") return "unknown";
  if (typeof run.lifecycle_state === "string" && run.lifecycle_state.trim()) return run.lifecycle_state;
  return "unknown";
};

export const formatRunSecondaryLabel = (run) =>
  `${deriveRunLifecycle(run)} | completed samples ${Number(run?.nr_completed_samples || 0).toLocaleString()}`;
