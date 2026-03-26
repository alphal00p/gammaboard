export const formatRunLabel = (run) => {
  if (!run) return "Unknown run";
  return run.run_name ? run.run_name : "Unnamed run";
};

export const deriveRunLifecycle = (run) => {
  if (!run || typeof run !== "object") return "unknown";
  if (typeof run.lifecycle_state === "string" && run.lifecycle_state.trim()) return run.lifecycle_state;
  const desiredAssignments = Number(run.desired_assignment_count);
  const activeWorkers = Number(run.active_worker_count);
  const claimedBatches = Number(run.claimed_batches);
  if (Number.isFinite(desiredAssignments) && desiredAssignments > 0) return "running";
  if (
    (Number.isFinite(activeWorkers) && activeWorkers > 0) ||
    (Number.isFinite(claimedBatches) && claimedBatches > 0)
  ) {
    return "pausing";
  }
  return "paused";
};

export const formatRunSecondaryLabel = (run) =>
  `${deriveRunLifecycle(run)} | completed samples ${Number(run?.nr_completed_samples || 0).toLocaleString()}`;
